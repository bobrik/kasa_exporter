//! # Kasa
//! A library for interacting with [TP-Link Kasa](https://www.kasasmart.com/) API

use std::fmt;
use std::sync;

pub mod error;

use crate::kasa::error::KasaError;

use anyhow::Result;

const ENDPOINT: &str = "https://wap.tplinkcloud.com/";

/// A client for interacting with API
pub struct Client<T> {
    client: hyper::Client<T>,
    app: String,
    username: String,
    password: String,
    token: sync::Mutex<String>,
}

impl<T> Client<T>
where
    T: hyper::client::connect::Connect + std::clone::Clone + std::marker::Send + Sync + 'static,
{
    /// Creates a new client with http client, credentials, and an app name (arbitrary string).
    pub async fn new(
        client: hyper::Client<T>,
        app: String,
        username: String,
        password: String,
    ) -> Result<Client<T>> {
        let token = Self::auth(&client, app.clone(), username.clone(), password.clone()).await?;

        Ok(Self {
            client,
            app,
            username,
            password,
            token: sync::Mutex::new(token),
        })
    }

    async fn auth(
        client: &hyper::Client<T>,
        app: String,
        username: String,
        password: String,
    ) -> Result<String> {
        let auth_response: Response<AuthResult> = Self::query(
            client,
            None,
            &Request {
                method: "login".to_string(),
                params: AuthParams::new(app, username, password),
            },
        )
        .await?;

        if let Some(result) = auth_response.result {
            Ok(result.token)
        } else {
            Err(KasaError::EmptyAuthResponse {
                code: auth_response.error_code,
                message: auth_response.message.unwrap_or_else(|| "".to_string()),
            }
            .into())
        }
    }

    /// Send a request to API with an optional token.
    async fn query<Q, R>(
        client: &hyper::Client<T>,
        token: Option<&String>,
        request: &Request<Q>,
    ) -> Result<Response<R>>
    where
        Q: serde::ser::Serialize + std::fmt::Debug,
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let request_body =
            serde_json::to_string(&request).map_err(|e| KasaError::Serialization {
                source: e.into(),
                debug: format!("{:?}", request),
            })?;

        let mut http_request = hyper::Request::new(hyper::Body::from(request_body));

        let mut uri = ENDPOINT.to_string();
        if let Some(value) = token {
            uri = uri + &"?token=".to_string() + value
        }

        let request_uri = uri.parse()?;

        *http_request.method_mut() = hyper::Method::POST;
        *http_request.uri_mut() = request_uri;

        http_request.headers_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );

        if cfg!(feature = "kasa_debug") {
            println!("> request:\n{:#?}", request);
        }

        let mut http_response = client.request(http_request).await?;

        let body = hyper::body::to_bytes(http_response.body_mut()).await?;

        let body_vec = body.to_vec();

        let resp = serde_json::from_slice(&body_vec).map_err(|e| {
            KasaError::Serialization {
                source: e.into(),
                debug: String::from_utf8(body_vec).unwrap_or_else(|e| e.to_string()),
            }
            .into()
        });

        if cfg!(feature = "kasa_debug") {
            println!("< response:\n{:#?}", resp);
        }

        resp
    }

    /// Sends an authenticated request with a token provided by auth request.
    async fn token_query<Q, R>(&self, req: &Request<Q>) -> Result<Response<R>>
    where
        Q: serde::ser::Serialize + std::fmt::Debug,
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let mut token = { self.token.lock().unwrap().clone() };

        let result = Self::query::<Q, R>(&self.client, Some(&token), req).await?;

        if result.error_code == -20675 || result.error_code == -20651 {
            token = Self::auth(
                &self.client,
                self.app.clone(),
                self.username.clone(),
                self.password.clone(),
            )
            .await?;

            let mut guarded_token = self.token.lock().unwrap();
            *guarded_token = token.clone();
        }

        Self::query::<Q, R>(&self.client, Some(&token), req).await
    }

    /// Sends a request directly to device via API.
    async fn passthrough_query<R>(
        &self,
        device_id: &str,
        req: &PassthroughParamsData,
    ) -> Result<Response<R>>
    where
        R: serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let params = PassthroughParams::new(device_id.to_owned(), req)
            .map_err(|e| KasaError::PassthroughParams { source: e.into() })?;

        self.token_query(&Request {
            method: "passthrough".to_string(),
            params,
        })
        .await
    }

    /// Returns list of devices available to the client.
    pub async fn get_device_list(&self) -> Result<Response<DeviceListResult>> {
        self.token_query(&Request {
            method: "getDeviceList".to_string(),
            params: DeviceListParams::new(),
        })
        .await
    }

    /// Returns emeter measurements from a supplied device.
    pub async fn emeter(&self, device_id: &str) -> Result<EmeterResult> {
        self.passthrough_query::<PassthroughResult>(
            device_id,
            &PassthroughParamsData::new().add_emeter(EmeterParams::new().add_realtime()),
        )
        .await?
        .result
        .ok_or(KasaError::EmptyPassthroughResponse {})?
        .unpack::<EmeterResultWrapper>()?
        .emeter
        .ok_or_else(|| KasaError::EmptyEmeterResponse {}.into())
    }
}

impl<T> fmt::Debug for Client<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Kasa {{ token: {} }}", self.token.lock().unwrap())
    }
}

/// A request to Kasa API.
#[derive(Debug, serde_derive::Serialize)]
struct Request<T> {
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
pub struct Response<T> {
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
    fn new<T: serde::ser::Serialize>(device_id: String, req: &T) -> serde_json::Result<Self> {
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
            KasaError::Serialization {
                source: e.into(),
                debug: self.response_data.clone(),
            }
            .into()
        })
    }
}

/// Parameters for passthrough requests.
#[derive(Debug, serde_derive::Serialize)]
struct PassthroughParamsData {
    #[serde(skip_serializing_if = "Option::is_none")]
    emeter: Option<EmeterParams>,
}

impl PassthroughParamsData {
    /// Creates empty passthrough parameters.
    fn new() -> Self {
        Self { emeter: None }
    }

    /// Adds query for emeter data.
    fn add_emeter(mut self, emeter: EmeterParams) -> Self {
        self.emeter = Some(emeter);
        self
    }
}

/// Parameters for emeter requests.
#[derive(Debug, serde_derive::Serialize)]
struct EmeterParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    get_realtime: Option<EmeterGetRealtimeParams>,
}

impl EmeterParams {
    /// Creates empty emeter parameters.
    fn new() -> Self {
        Self { get_realtime: None }
    }

    /// Adds query for realtime data.
    fn add_realtime(mut self) -> Self {
        self.get_realtime = Some(EmeterGetRealtimeParams {});
        self
    }
}

/// Parameters for realtime emeter data.
#[derive(Debug, serde_derive::Serialize)]
struct EmeterGetRealtimeParams {}

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
    pub total: Option<f64>,
}
