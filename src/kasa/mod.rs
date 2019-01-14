use std::error::Error;
use std::fmt;

use futures::future;
use futures::future::Future;
use futures::stream::Stream;

use hyper::client::connect::HttpConnector;
use hyper::Body;
use hyper::Client;
use hyper::Method;
use hyper::Request;

use hyper_tls::HttpsConnector;

use uuid::Uuid;

const ENDPOINT: &str = "https://wap.tplinkcloud.com/";

pub struct Kasa {
    client: Client<HttpsConnector<HttpConnector>>,
    token: String,
}

impl Kasa {
    pub fn new(
        app: String,
        username: String,
        password: String,
    ) -> impl Future<Item = Kasa, Error = KasaError> {
        let client = match Self::client() {
            Err(e) => return future::Either::A(future::err(e)),
            Ok(client) => client,
        };

        future::Either::B(
            Self::query(
                &client,
                None,
                KasaRequest {
                    method: "login".to_string(),
                    params: AuthParams::new(app, username, password),
                },
            )
            .and_then(|auth_response: KasaResponse<AuthResult>| {
                if let Some(result) = auth_response.result {
                    future::ok(Self {
                        client,
                        token: result.token,
                    })
                } else {
                    future::err(KasaError::new(
                        auth_response.error_code,
                        auth_response
                            .message
                            .unwrap_or("auth response is empty".to_string()),
                    ))
                }
            }),
        )
    }

    fn client() -> Result<Client<HttpsConnector<HttpConnector>>, KasaError> {
        match HttpsConnector::new(4) {
            Ok(connector) => Ok(Client::builder().build::<_, Body>(connector)),
            Err(e) => Err(KasaError::new(
                0,
                format!(
                    "cannot create https connector: {}",
                    e.description().to_string()
                ),
            )),
        }
    }

    fn query<Q, R>(
        client: &Client<HttpsConnector<HttpConnector>>,
        token: Option<&String>,
        request: KasaRequest<Q>,
    ) -> impl Future<Item = KasaResponse<R>, Error = KasaError>
    where
        Q: serde::ser::Serialize + std::fmt::Debug,
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let request_body = match serde_json::to_string(&request) {
            Err(e) => {
                return future::Either::A(future::err(KasaError::new(
                    0,
                    format!("error serializing request body: {:}", e),
                )))
            }
            Ok(request_body) => request_body,
        };

        let mut http_request = Request::new(Body::from(request_body));

        let mut uri = ENDPOINT.to_string();
        if let Some(value) = token {
            uri = uri + &"?token=".to_string() + &value
        }

        let request_uri = match uri.parse() {
            Err(e) => {
                return future::Either::A(future::err(KasaError::new(
                    0,
                    format!("error parsing request uri: {:}", e),
                )))
            }
            Ok(request_uri) => request_uri,
        };

        *http_request.method_mut() = Method::POST;
        *http_request.uri_mut() = request_uri;

        http_request.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );

        if cfg!(feature = "kasa_debug") {
            println!("> request:\n{:#?}", request);
        }

        future::Either::B(
            client
                .request(http_request)
                .and_then(|http_response| http_response.into_body().concat2())
                .map_err(|e| {
                    KasaError::new(
                        0,
                        format!(
                            "error while talking to kasa api: {}",
                            e.description().to_string()
                        ),
                    )
                })
                .and_then(|body| match serde_json::from_slice(&body.to_vec()) {
                    Ok(resp) => future::ok(resp),
                    Err(e) => future::err(KasaError::new(
                        0,
                        format!("error parsing response: {}", e.description().to_string()),
                    )),
                })
                .map(|resp| {
                    if cfg!(feature = "kasa_debug") {
                        println!("< response:\n{:#?}", resp);
                    }
                    resp
                }),
        )
    }

    fn token_query<Q, R>(
        &self,
        req: KasaRequest<Q>,
    ) -> impl Future<Item = KasaResponse<R>, Error = KasaError>
    where
        Q: serde::ser::Serialize + std::fmt::Debug,
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        Self::query(&self.client, Some(&self.token), req)
    }

    fn passthrough_query<R>(
        &self,
        device_id: &String,
        req: &PassthroughParamsData,
    ) -> impl Future<Item = KasaResponse<R>, Error = KasaError>
    where
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        match PassthroughParams::new(device_id.to_owned(), req) {
            Ok(params) => future::Either::A(self.token_query(KasaRequest {
                method: "passthrough".to_string(),
                params: params,
            })),
            Err(e) => future::Either::B(future::err(KasaError::new(
                0,
                format!("error serializing passthrough params: {}", e.description()),
            ))),
        }
    }

    pub fn get_device_list(
        &self,
    ) -> impl Future<Item = KasaResponse<DeviceListResult>, Error = KasaError> {
        self.token_query(KasaRequest {
            method: "getDeviceList".to_string(),
            params: DeviceListParams::new(),
        })
    }

    pub fn emeter(
        &self,
        device_id: &String,
    ) -> impl Future<Item = EmeterResult, Error = KasaError> {
        self.passthrough_query(
            device_id,
            &PassthroughParamsData::new().add_emeter(EMeterParams::new().add_realtime()),
        )
        .and_then(
            |response: KasaResponse<PassthroughResult>| match response.result {
                Some(result) => match result.unpack() {
                    Ok(emeter) => future::ok(emeter),
                    Err(e) => future::err(KasaError::new(
                        0,
                        format!(
                            "error parsing passthrough response: {}",
                            e.description().to_string()
                        ),
                    )),
                },
                None => future::err(KasaError::new(
                    response.error_code,
                    response.message.unwrap_or("response is empty".to_string()),
                )),
            },
        )
        .and_then(|w: EmeterResultWrapper| match w.emeter {
            Some(emeter) => future::ok(emeter),
            None => future::err(KasaError::new(0, "emeter response is empty".to_string())),
        })
    }
}

impl fmt::Debug for Kasa {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Kasa {{ token: {} }}", self.token)
    }
}

#[derive(Debug, serde_derive::Serialize)]
struct KasaRequest<T> {
    method: String,
    params: T,
}

#[derive(Debug, serde_derive::Serialize)]
struct AuthParams {
    #[serde(rename = "appType")]
    app_type: String,

    #[serde(rename = "cloudUserName")]
    cloud_user_name: String,

    #[serde(rename = "cloudPassword")]
    cloud_password: String,

    #[serde(rename = "terminalUUID")]
    terminal_uuid: String,
}

impl AuthParams {
    fn new(app_type: String, username: String, password: String) -> Self {
        return Self {
            app_type,
            cloud_user_name: username,
            cloud_password: password,
            terminal_uuid: Uuid::new_v4().to_string(),
        };
    }
}

#[derive(Debug, serde_derive::Deserialize)]
pub struct KasaResponse<T> {
    pub error_code: i32,
    #[serde(rename = "msg")]
    pub message: Option<String>,
    pub result: Option<T>,
}

#[derive(Debug, serde_derive::Deserialize)]
struct AuthResult {
    #[serde(rename = "accountId")]
    account_id: String,

    email: String,

    token: String,
}

#[derive(Debug, serde_derive::Serialize)]
struct DeviceListParams {}

impl DeviceListParams {
    fn new() -> Self {
        return Self {};
    }
}

#[derive(Debug, serde_derive::Deserialize)]
pub struct DeviceListResult {
    #[serde(rename = "deviceList")]
    pub device_list: Vec<DeviceListEntry>,
}

#[derive(Debug, serde_derive::Deserialize)]
pub struct DeviceListEntry {
    pub alias: String,

    pub status: i32,

    #[serde(rename = "deviceModel")]
    pub model: String,

    #[serde(rename = "deviceId")]
    pub device_id: String,

    #[serde(rename = "deviceHwVer")]
    pub hardware_version: String,

    #[serde(rename = "fwVer")]
    pub firmware_version: String,
}

#[derive(Debug, serde_derive::Serialize)]
struct PassthroughParams {
    #[serde(rename = "deviceId")]
    device_id: String,

    #[serde(rename = "requestData")]
    request_data: String,
}

impl PassthroughParams {
    fn new<T: serde::ser::Serialize>(
        device_id: String,
        req: &T,
    ) -> Result<Self, serde_json::Error> {
        let request_data = serde_json::to_string(req)?;

        Ok(Self {
            device_id,
            request_data,
        })
    }
}

#[derive(Debug, serde_derive::Deserialize)]
pub struct PassthroughResult {
    #[serde(rename = "responseData")]
    response_data: String,
}

impl PassthroughResult {
    fn unpack<T>(&self) -> Result<T, serde_json::Error>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_str(&self.response_data)
    }
}

#[derive(Debug, serde_derive::Serialize)]
struct PassthroughParamsData {
    #[serde(skip_serializing_if = "Option::is_none")]
    emeter: Option<EMeterParams>,
}

impl PassthroughParamsData {
    fn new() -> Self {
        Self { emeter: None }
    }

    fn add_emeter(mut self, emeter: EMeterParams) -> Self {
        self.emeter = Some(emeter);
        self
    }
}

#[derive(Debug, serde_derive::Serialize)]
struct EMeterParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    get_realtime: Option<EMeterGetRealtimeParams>,
}

impl EMeterParams {
    fn new() -> Self {
        Self { get_realtime: None }
    }

    fn add_realtime(mut self) -> Self {
        self.get_realtime = Some(EMeterGetRealtimeParams {});
        self
    }
}

#[derive(Debug, serde_derive::Serialize)]
struct EMeterGetRealtimeParams {}

#[derive(Debug, serde_derive::Deserialize)]
struct EmeterResultWrapper {
    emeter: Option<EmeterResult>,
}

#[derive(Debug, serde_derive::Deserialize)]
pub struct EmeterResult {
    pub get_realtime: Option<EmeterGetRealtimeResult>,
}

#[derive(Debug, serde_derive::Deserialize)]
pub struct EmeterGetRealtimeResult {
    pub error_code: Option<i32>,
    pub current: Option<f64>,
    pub voltage: Option<f64>,
    pub power: Option<f64>,
}

#[derive(Debug)]
pub struct KasaError {
    pub code: i32,
    pub message: String,
}

// TODO: handle error chains
impl KasaError {
    fn new(code: i32, message: String) -> Self {
        return Self { code, message };
    }
}

impl Error for KasaError {
    fn description(&self) -> &str {
        self.message.as_str()
    }

    fn cause(&self) -> Option<&Error> {
        None
    }
}

impl fmt::Display for KasaError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Kasa error (code = {}, message = {})",
            self.code, self.message
        )
    }
}
