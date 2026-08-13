use std::fs;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use reqwest::{Method, StatusCode, header::HeaderMap};
use rsa::{
    Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey,
    pkcs1::DecodeRsaPrivateKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::config::WechatPayConfig;

const WECHATPAY_API_BASE_URL: &str = "https://api.mch.weixin.qq.com";
const WECHATPAY_SERIAL_HEADER: &str = "Wechatpay-Serial";

#[derive(Debug, Error)]
pub enum WechatPayError {
    #[error("failed to read WeChat Pay key file: {0}")]
    KeyFile(#[from] std::io::Error),
    #[error("invalid WeChat Pay key: {0}")]
    InvalidKey(String),
    #[error("WeChat Pay HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WeChat Pay message serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("missing WeChat Pay signature header: {0}")]
    MissingSignatureHeader(&'static str),
    #[error("unexpected WeChat Pay public key id: {0}")]
    UnexpectedPublicKeyId(String),
    #[error("invalid WeChat Pay signature encoding")]
    InvalidSignatureEncoding,
    #[error("WeChat Pay signature verification failed")]
    SignatureVerification,
    #[error("WeChat Pay cryptographic operation failed: {0}")]
    Crypto(String),
    #[error("WeChat Pay API returned HTTP {status}: {code} - {message}")]
    Api {
        status: StatusCode,
        code: String,
        message: String,
    },
    #[error("invalid WeChat Pay response: {0}")]
    InvalidResponse(String),
}

#[derive(Clone)]
pub struct WechatPayClient {
    app_id: String,
    mch_id: String,
    merchant_serial_no: String,
    api_v3_key: [u8; 32],
    public_key_id: String,
    private_key: RsaPrivateKey,
    public_key: RsaPublicKey,
    notify_url: String,
    http: reqwest::Client,
}

#[derive(Debug, Serialize)]
pub struct NativePrepayRequest<'a> {
    pub appid: &'a str,
    pub mchid: &'a str,
    pub description: &'a str,
    pub out_trade_no: &'a str,
    pub time_expire: String,
    pub attach: String,
    pub notify_url: &'a str,
    pub amount: WechatPayAmountRequest,
}

#[derive(Debug, Serialize)]
pub struct WechatPayAmountRequest {
    pub total: i64,
    pub currency: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct NativePrepayResponse {
    pub code_url: String,
}

#[derive(Debug, Deserialize)]
pub struct WechatPayNotification {
    pub id: String,
    pub event_type: String,
    pub resource: EncryptedResource,
}

#[derive(Debug, Deserialize)]
pub struct EncryptedResource {
    pub algorithm: String,
    pub ciphertext: String,
    pub associated_data: Option<String>,
    pub nonce: String,
    pub original_type: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WechatPayTransaction {
    pub appid: String,
    pub mchid: String,
    pub out_trade_no: String,
    pub transaction_id: String,
    pub trade_type: String,
    pub trade_state: String,
    pub success_time: Option<String>,
    pub attach: Option<String>,
    pub amount: WechatPayTransactionAmount,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WechatPayTransactionAmount {
    pub total: i64,
    pub payer_total: Option<i64>,
    pub currency: String,
    pub payer_currency: Option<String>,
}

/// 主动查单在 `NOTPAY`、`CLOSED` 等状态下不会返回完整的成功交易字段，不能直接
/// 反序列化成支付成功通知模型。只有状态为 `SUCCESS` 时才通过 `into_success` 收紧。
#[derive(Debug, Deserialize)]
pub struct WechatPayQueryTransaction {
    pub appid: String,
    pub mchid: String,
    pub out_trade_no: String,
    pub transaction_id: Option<String>,
    pub trade_type: Option<String>,
    pub trade_state: String,
    pub success_time: Option<String>,
    pub attach: Option<String>,
    pub amount: Option<WechatPayTransactionAmount>,
}

impl WechatPayQueryTransaction {
    pub fn into_success(self) -> Result<WechatPayTransaction, WechatPayError> {
        if self.trade_state != "SUCCESS" {
            return Err(WechatPayError::InvalidResponse(format!(
                "query result is not successful: {}",
                self.trade_state
            )));
        }
        Ok(WechatPayTransaction {
            appid: self.appid,
            mchid: self.mchid,
            out_trade_no: self.out_trade_no,
            transaction_id: self.transaction_id.ok_or_else(|| {
                WechatPayError::InvalidResponse(
                    "successful query is missing transaction_id".to_string(),
                )
            })?,
            trade_type: self.trade_type.ok_or_else(|| {
                WechatPayError::InvalidResponse(
                    "successful query is missing trade_type".to_string(),
                )
            })?,
            trade_state: self.trade_state,
            success_time: self.success_time,
            attach: self.attach,
            amount: self.amount.ok_or_else(|| {
                WechatPayError::InvalidResponse("successful query is missing amount".to_string())
            })?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct WechatPayApiErrorBody {
    code: Option<String>,
    message: Option<String>,
}

impl WechatPayClient {
    pub fn from_config(config: &WechatPayConfig) -> Result<Self, WechatPayError> {
        let private_key_pem = fs::read_to_string(&config.merchant_private_key_file)?;
        let public_key_pem = fs::read_to_string(&config.public_key_file)?;
        let private_key = RsaPrivateKey::from_pkcs8_pem(&private_key_pem)
            .or_else(|_| RsaPrivateKey::from_pkcs1_pem(&private_key_pem))
            .map_err(|error| WechatPayError::InvalidKey(error.to_string()))?;
        let public_key = RsaPublicKey::from_public_key_pem(&public_key_pem)
            .map_err(|error| WechatPayError::InvalidKey(error.to_string()))?;
        let api_v3_key: [u8; 32] =
            config.api_v3_key.as_bytes().try_into().map_err(|_| {
                WechatPayError::InvalidKey("APIv3 key must be 32 bytes".to_string())
            })?;
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("qddxp-wechatpay/1.0")
            .build()?;

        tracing::info!(
            app_id = %config.app_id,
            mch_id = %config.mch_id,
            merchant_serial_no = %config.merchant_serial_no,
            public_key_id = %config.public_key_id,
            "official WeChat Pay API v3 client initialized"
        );
        Ok(Self {
            app_id: config.app_id.clone(),
            mch_id: config.mch_id.clone(),
            merchant_serial_no: config.merchant_serial_no.clone(),
            api_v3_key,
            public_key_id: config.public_key_id.clone(),
            private_key,
            public_key,
            notify_url: config.notify_url.clone(),
            http,
        })
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    pub fn mch_id(&self) -> &str {
        &self.mch_id
    }

    pub fn notify_url(&self) -> &str {
        &self.notify_url
    }

    pub async fn native_prepay(
        &self,
        description: &str,
        merchant_trade_no: &str,
        order_id: Uuid,
        amount_cents: i64,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<NativePrepayResponse, WechatPayError> {
        // 微信 Native 下单的商品描述上限为 127 个字符。商品名称来自后台配置，
        // 在协议边界统一裁剪，避免超长名称造成明知可预防的远端下单失败。
        let description = description.trim().chars().take(127).collect::<String>();
        if description.is_empty() {
            return Err(WechatPayError::InvalidResponse(
                "payment description must not be empty".to_string(),
            ));
        }
        let request = NativePrepayRequest {
            appid: &self.app_id,
            mchid: &self.mch_id,
            description: &description,
            out_trade_no: merchant_trade_no,
            time_expire: expires_at.to_rfc3339(),
            attach: order_id.to_string(),
            notify_url: &self.notify_url,
            amount: WechatPayAmountRequest {
                total: amount_cents,
                currency: "CNY",
            },
        };
        let value = serde_json::to_value(&request)?;
        let response = self
            .execute_json::<_, NativePrepayResponse>(
                Method::POST,
                "/v3/pay/transactions/native",
                Some(&value),
            )
            .await?;
        if response.code_url.trim().is_empty() {
            return Err(WechatPayError::InvalidResponse(
                "Native prepay response contains an empty code_url".to_string(),
            ));
        }
        Ok(response)
    }

    pub async fn query_order(
        &self,
        merchant_trade_no: &str,
    ) -> Result<WechatPayQueryTransaction, WechatPayError> {
        let path = format!(
            "/v3/pay/transactions/out-trade-no/{merchant_trade_no}?mchid={}",
            self.mch_id
        );
        self.execute_json::<serde_json::Value, WechatPayQueryTransaction>(Method::GET, &path, None)
            .await
    }

    pub async fn close_order(&self, merchant_trade_no: &str) -> Result<(), WechatPayError> {
        let path = format!("/v3/pay/transactions/out-trade-no/{merchant_trade_no}/close");
        let body = serde_json::json!({ "mchid": self.mch_id });
        self.execute_empty(Method::POST, &path, Some(&body)).await
    }

    /// 验证微信支付应答或回调的签名。调用方必须传入未经解析、未经重排的原始报文。
    pub fn verify_signed_message(
        &self,
        headers: &HeaderMap,
        body: &str,
    ) -> Result<(), WechatPayError> {
        let timestamp = signature_header(headers, "Wechatpay-Timestamp")?;
        let nonce = signature_header(headers, "Wechatpay-Nonce")?;
        let signature = signature_header(headers, "Wechatpay-Signature")?;
        let serial = signature_header(headers, "Wechatpay-Serial")?;
        if serial != self.public_key_id {
            return Err(WechatPayError::UnexpectedPublicKeyId(serial.to_string()));
        }
        let message = format!("{timestamp}\n{nonce}\n{body}\n");
        let digest = Sha256::digest(message.as_bytes());
        let signature = BASE64_STANDARD
            .decode(signature)
            .map_err(|_| WechatPayError::InvalidSignatureEncoding)?;
        self.public_key
            .verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
            .map_err(|_| WechatPayError::SignatureVerification)
    }

    pub fn decrypt_notification<T: DeserializeOwned>(
        &self,
        resource: &EncryptedResource,
    ) -> Result<T, WechatPayError> {
        if resource.algorithm != "AEAD_AES_256_GCM" {
            return Err(WechatPayError::InvalidResponse(format!(
                "unsupported resource algorithm: {}",
                resource.algorithm
            )));
        }
        let nonce: [u8; 12] =
            resource.nonce.as_bytes().try_into().map_err(|_| {
                WechatPayError::Crypto("notification nonce must be 12 bytes".into())
            })?;
        let ciphertext = BASE64_STANDARD
            .decode(&resource.ciphertext)
            .map_err(|error| WechatPayError::Crypto(error.to_string()))?;
        let cipher = Aes256Gcm::new_from_slice(&self.api_v3_key)
            .map_err(|error| WechatPayError::Crypto(error.to_string()))?;
        let plaintext = cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &ciphertext,
                    aad: resource.associated_data.as_deref().unwrap_or("").as_bytes(),
                },
            )
            .map_err(|error| WechatPayError::Crypto(error.to_string()))?;
        Ok(serde_json::from_slice(&plaintext)?)
    }

    async fn execute_json<B: Serialize + ?Sized, R: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<R, WechatPayError> {
        let response_body = self.execute(method, path, body).await?;
        Ok(serde_json::from_str(&response_body)?)
    }

    async fn execute_empty<B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<(), WechatPayError> {
        self.execute(method, path, body).await.map(|_| ())
    }

    async fn execute<B: Serialize + ?Sized>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<String, WechatPayError> {
        let body = body
            .map(serde_json::to_string)
            .transpose()?
            .unwrap_or_default();
        let timestamp = chrono::Utc::now().timestamp();
        let nonce = Uuid::new_v4().simple().to_string();
        let canonical = canonical_request_message(method.as_str(), path, timestamp, &nonce, &body);
        let digest = Sha256::digest(canonical.as_bytes());
        let signature = self
            .private_key
            .sign(Pkcs1v15Sign::new::<Sha256>(), &digest)
            .map_err(|error| WechatPayError::Crypto(error.to_string()))?;
        let authorization = format!(
            "WECHATPAY2-SHA256-RSA2048 mchid=\"{}\",nonce_str=\"{}\",timestamp=\"{}\",serial_no=\"{}\",signature=\"{}\"",
            self.mch_id,
            nonce,
            timestamp,
            self.merchant_serial_no,
            BASE64_STANDARD.encode(signature)
        );
        tracing::info!(
            method = method.as_str(),
            path,
            has_body = !body.is_empty(),
            wechatpay_public_key_id = %self.public_key_id,
            "sending official WeChat Pay API v3 request"
        );

        let response = self
            .build_http_request(method.clone(), path, authorization, body)
            .send()
            .await?;
        let status = response.status();
        let headers = response.headers().clone();
        let response_body = response.text().await?;

        self.verify_signed_message(&headers, &response_body)?;
        tracing::info!(
            method = method.as_str(),
            path,
            status = status.as_u16(),
            "official WeChat Pay API v3 response signature verified"
        );
        if !status.is_success() {
            let error = serde_json::from_str::<WechatPayApiErrorBody>(&response_body).unwrap_or(
                WechatPayApiErrorBody {
                    code: None,
                    message: None,
                },
            );
            return Err(WechatPayError::Api {
                status,
                code: error.code.unwrap_or_else(|| "UNKNOWN".to_string()),
                message: error
                    .message
                    .unwrap_or_else(|| "unknown WeChat Pay API error".to_string()),
            });
        }
        Ok(response_body)
    }

    fn build_http_request(
        &self,
        method: Method,
        path: &str,
        authorization: String,
        body: String,
    ) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{WECHATPAY_API_BASE_URL}{path}"))
            .header(reqwest::header::AUTHORIZATION, authorization)
            // 公钥模式必须携带微信支付公钥 ID。它与 Authorization 中的商户证书
            // serial_no 含义不同，统一在底层添加可避免下单、查单、关单接口遗漏。
            .header(WECHATPAY_SERIAL_HEADER, &self.public_key_id)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
    }
}

fn signature_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, WechatPayError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or(WechatPayError::MissingSignatureHeader(name))
}

pub(crate) fn canonical_request_message(
    method: &str,
    path: &str,
    timestamp: i64,
    nonce: &str,
    body: &str,
) -> String {
    format!("{method}\n{path}\n{timestamp}\n{nonce}\n{body}\n")
}
