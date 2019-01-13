use std::fmt;

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
    ) -> impl Future<Item = Kasa, Error = ()> {
        let client = Self::client();

        Self::query(
            &client,
            None,
            KasaRequest {
                method: "login".to_string(),
                params: AuthParams::new(app, username, password),
            },
        )
        .map(|auth_response: KasaResponse<AuthResult>| Self {
            client,
            token: auth_response.result.unwrap().token,
        })
    }

    fn client() -> Client<HttpsConnector<HttpConnector>> {
        Client::builder().build::<_, Body>(HttpsConnector::new(4).unwrap())
    }

    fn query<Q, R>(
        client: &Client<HttpsConnector<HttpConnector>>,
        token: Option<&String>,
        req: KasaRequest<Q>,
    ) -> impl Future<Item = KasaResponse<R>, Error = ()>
    where
        Q: serde::ser::Serialize + std::fmt::Debug,
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let mut http_req = Request::new(Body::from(serde_json::to_string(&req).unwrap()));

        let mut uri = ENDPOINT.to_string();
        if let Some(value) = token {
            uri = uri + &"?token=".to_string() + &value
        }

        *http_req.method_mut() = Method::POST;
        *http_req.uri_mut() = uri.parse().unwrap();

        http_req.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );

        if cfg!(feature = "kasa_debug") {
            println!("> request:\n{:#?}", req);
        }

        client
            .request(http_req)
            .and_then(|res| res.into_body().concat2())
            .map(|done| {
                let resp = serde_json::from_slice(&done.to_vec()).unwrap();
                if cfg!(feature = "kasa_debug") {
                    println!("< response:\n{:#?}", resp);
                }
                resp
            })
            .map_err(|err| {
                // TODO: handle error
                println!("Error: {}", err);
            })
    }

    fn token_query<Q, R>(
        &self,
        req: KasaRequest<Q>,
    ) -> impl Future<Item = KasaResponse<R>, Error = ()>
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
    ) -> impl Future<Item = KasaResponse<R>, Error = ()>
    where
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        self.token_query(KasaRequest {
            method: "passthrough".to_string(),
            params: PassthroughParams::new(device_id.to_owned(), req),
        })
    }

    pub fn get_device_list(
        &self,
    ) -> impl Future<Item = KasaResponse<DeviceListResult>, Error = ()> {
        self.token_query(KasaRequest {
            method: "getDeviceList".to_string(),
            params: DeviceListParams::new(),
        })
    }

    pub fn emeter(&self, device_id: &String) -> impl Future<Item = EmeterResult, Error = ()> {
        self.passthrough_query(
            device_id,
            &PassthroughParamsData::new().add_emeter(EMeterParams::new().add_realtime()),
        )
        .map(
            |response: KasaResponse<PassthroughResult>| -> EmeterResultWrapper {
                response.result.unwrap().unpack().unwrap()
            },
        )
        .map(|w| w.emeter.unwrap())
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
    pub msg: Option<String>,
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
    fn new<T: serde::ser::Serialize>(device_id: String, req: &T) -> Self {
        Self {
            device_id,
            request_data: serde_json::to_string(req).unwrap(),
        }
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
