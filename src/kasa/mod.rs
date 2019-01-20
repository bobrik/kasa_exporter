//! # Kasa
//! A library for interacting with [TP-Link Kasa](https://www.kasasmart.com/) API

use std::error::Error as StdError;
use std::fmt;

use futures::future;
use futures::future::Future;
use futures::stream::Stream;

use hyper::client::connect::HttpConnector;
use hyper::{Body, Client, Method, Request};

use hyper_tls::HttpsConnector;

use uuid;

mod error;

use crate::kasa::error::*;

const ENDPOINT: &str = "https://wap.tplinkcloud.com/";

/// A client for interacting with API
pub struct Kasa {
    client: Client<HttpsConnector<HttpConnector>>,
    token: String,
}

impl Kasa {
    /// Creates a new client with credentials and app name (arbitrary string).
    ///
    /// This method returns a future and should be called in a [tokio](https://tokio.rs) runtime.
    pub fn new(
        app: String,
        username: String,
        password: String,
    ) -> impl Future<Item = Kasa, Error = Error> {
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
                    future::err(
                        ErrorKind::EmptyAuthResponse(
                            auth_response.error_code,
                            auth_response.message.unwrap_or_else(|| "".to_string()),
                        )
                        .into(),
                    )
                }
            }),
        )
    }

    /// Returns an HTTPS client for network communication.
    fn client() -> Result<Client<HttpsConnector<HttpConnector>>> {
        Ok(Client::builder().build::<_, Body>(HttpsConnector::new(4)?))
    }

    /// Send a request to API with an optional token.
    fn query<Q, R>(
        client: &Client<HttpsConnector<HttpConnector>>,
        token: Option<&String>,
        request: KasaRequest<Q>,
    ) -> impl Future<Item = KasaResponse<R>, Error = Error>
    where
        Q: serde::ser::Serialize + std::fmt::Debug,
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let request_body = match serde_json::to_string(&request)
            .map_err(|e| Error::with_chain(e, ErrorKind::Serialization(format!("{:?}", request))))
        {
            Err(e) => return future::Either::A(future::err(e)),
            Ok(request_body) => request_body,
        };

        let mut http_request = Request::new(Body::from(request_body));

        let mut uri = ENDPOINT.to_string();
        if let Some(value) = token {
            uri = uri + &"?token=".to_string() + &value
        }

        let request_uri = match uri.parse().map_err(From::from) {
            Err(e) => return future::Either::A(future::err(e)),
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
                .from_err::<Error>()
                .and_then(|http_response| http_response.into_body().concat2().from_err())
                .and_then(|body| {
                    let body_vec = body.to_vec();
                    serde_json::from_slice(&body_vec).map_err(|e| {
                        Error::with_chain(
                            e,
                            ErrorKind::Serialization(
                                String::from_utf8(body_vec)
                                    .unwrap_or_else(|e| e.description().to_string()),
                            ),
                        )
                    })
                })
                .map(|resp| {
                    if cfg!(feature = "kasa_debug") {
                        println!("< response:\n{:#?}", resp);
                    }
                    resp
                }),
        )
    }

    /// Sends an authenticated request with a token provided by auth request.
    fn token_query<Q, R>(
        &self,
        req: KasaRequest<Q>,
    ) -> impl Future<Item = KasaResponse<R>, Error = Error>
    where
        Q: serde::ser::Serialize + std::fmt::Debug,
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        Self::query(&self.client, Some(&self.token), req)
    }

    /// Sends a request directly to device via API.
    fn passthrough_query<R>(
        &self,
        device_id: &str,
        req: &PassthroughParamsData,
    ) -> impl Future<Item = KasaResponse<R>, Error = Error>
    where
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        match PassthroughParams::new(device_id.to_owned(), req) {
            Ok(params) => future::Either::A(self.token_query(KasaRequest {
                method: "passthrough".to_string(),
                params,
            })),
            Err(e) => future::Either::B(future::err(e.chain_err(|| ErrorKind::PassthtoughParams))),
        }
    }

    /// Returns list of devices available to the client.
    pub fn get_device_list(
        &self,
    ) -> impl Future<Item = KasaResponse<DeviceListResult>, Error = Error> {
        self.token_query(KasaRequest {
            method: "getDeviceList".to_string(),
            params: DeviceListParams::new(),
        })
    }

    /// Returns emeter measurements from a supplied device.
    pub fn emeter(&self, device_id: &str) -> impl Future<Item = EmeterResult, Error = Error> {
        self.passthrough_query(
            device_id,
            &PassthroughParamsData::new().add_emeter(EMeterParams::new().add_realtime()),
        )
        .and_then(
            |response: KasaResponse<PassthroughResult>| match response.result {
                Some(result) => match result.unpack() {
                    Ok(emeter) => future::ok(emeter),
                    Err(e) => future::err(e),
                },
                None => future::err(ErrorKind::EmptyPassthroughResponse.into()),
            },
        )
        .and_then(|w: EmeterResultWrapper| match w.emeter {
            Some(emeter) => future::ok(emeter),
            None => future::err(ErrorKind::EmptyEmeterResponse.into()),
        })
    }
}

impl fmt::Debug for Kasa {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Kasa {{ token: {} }}", self.token)
    }
}

/// A request to Kasa API.
#[derive(Debug, serde_derive::Serialize)]
struct KasaRequest<T> {
    method: String,
    params: T,
}

/// Parameters for authentication request.
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
    /// Creates authentication request parameters with given credentials.
    fn new(app_type: String, username: String, password: String) -> Self {
        Self {
            app_type,
            cloud_user_name: username,
            cloud_password: password,
            terminal_uuid: uuid::Uuid::new_v4().to_string(),
        }
    }
}

/// A generic response from Kasa API.
#[derive(Debug, serde_derive::Deserialize)]
pub struct KasaResponse<T> {
    pub error_code: i32,
    #[serde(rename = "msg")]
    pub message: Option<String>,
    pub result: Option<T>,
}

/// An authentication response data.
#[derive(Debug, serde_derive::Deserialize)]
struct AuthResult {
    #[serde(rename = "accountId")]
    account_id: String,

    email: String,

    token: String,
}

/// Parameters for device list request.
#[derive(Debug, serde_derive::Serialize)]
struct DeviceListParams {}

impl DeviceListParams {
    /// Creates empty device list parameters.
    fn new() -> Self {
        Self {}
    }
}

/// List of devices.
#[derive(Debug, serde_derive::Deserialize)]
pub struct DeviceListResult {
    #[serde(rename = "deviceList")]
    pub device_list: Vec<DeviceListEntry>,
}

/// Device data from listing response.
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

/// A wrapper for parameters for passthrough (going directly to device) requests.
#[derive(Debug, serde_derive::Serialize)]
struct PassthroughParams {
    #[serde(rename = "deviceId")]
    device_id: String,

    #[serde(rename = "requestData")]
    request_data: String,
}

impl PassthroughParams {
    /// Creates empty passthrough parameters.
    fn new<T: serde::ser::Serialize>(device_id: String, req: &T) -> Result<Self> {
        let request_data = serde_json::to_string(req)?;

        Ok(Self {
            device_id,
            request_data,
        })
    }
}

/// Response data for passthrough requests.
#[derive(Debug, serde_derive::Deserialize)]
pub struct PassthroughResult {
    #[serde(rename = "responseData")]
    response_data: String,
}

impl PassthroughResult {
    /// Unpacks double-encoded passthrough result.
    fn unpack<T>(&self) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_str(&self.response_data).map_err(|e| {
            Error::with_chain(e, ErrorKind::Deserialization(self.response_data.clone()))
        })
    }
}

/// Parameters for passthrough requests.
#[derive(Debug, serde_derive::Serialize)]
struct PassthroughParamsData {
    #[serde(skip_serializing_if = "Option::is_none")]
    emeter: Option<EMeterParams>,
}

impl PassthroughParamsData {
    /// Creates empty passthrough parameters.
    fn new() -> Self {
        Self { emeter: None }
    }

    /// Adds query for emeter data.
    fn add_emeter(mut self, emeter: EMeterParams) -> Self {
        self.emeter = Some(emeter);
        self
    }
}

/// Parameters for emeter requests.
#[derive(Debug, serde_derive::Serialize)]
struct EMeterParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    get_realtime: Option<EMeterGetRealtimeParams>,
}

impl EMeterParams {
    /// Creates empty emeter parameters.
    fn new() -> Self {
        Self { get_realtime: None }
    }

    /// Adds query for realtime data.
    fn add_realtime(mut self) -> Self {
        self.get_realtime = Some(EMeterGetRealtimeParams {});
        self
    }
}

/// Parameters for realtime emeter data.
#[derive(Debug, serde_derive::Serialize)]
struct EMeterGetRealtimeParams {}

/// A wrapper for emeter results.
#[derive(Debug, serde_derive::Deserialize)]
struct EmeterResultWrapper {
    emeter: Option<EmeterResult>,
}

/// Results of an emeter request.
#[derive(Debug, serde_derive::Deserialize)]
pub struct EmeterResult {
    pub get_realtime: Option<EmeterGetRealtimeResult>,
}

/// Realtime measurements from an emeter request.
#[derive(Debug, serde_derive::Deserialize)]
pub struct EmeterGetRealtimeResult {
    pub error_code: Option<i32>,
    pub current: Option<f64>,
    pub voltage: Option<f64>,
    pub power: Option<f64>,
}
