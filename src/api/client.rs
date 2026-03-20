use anyhow::{Context, Result};
use reqwest::{Client, Response, StatusCode};
use serde::de::DeserializeOwned;
use tracing::{debug, instrument};

use crate::config::TokenStorage;

/// Base URL for TickTick Open API v1
pub const API_BASE_URL: &str = "https://api.ticktick.com/open/v1";

/// Base URL for TickTick Internal API v2
pub const API_V2_BASE_URL: &str = "https://api.ticktick.com/api/v2";

/// TickTick API client wrapper
#[derive(Debug, Clone)]
pub struct TickTickClient {
    client: Client,
    token: String,
    base_url: String,
    /// Session cookies for v2 API authentication
    session_cookies: Option<String>,
    /// X-Device header for v2 API
    x_device: String,
}

/// API error response from TickTick
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Authentication required. Run 'tickrs init' to authenticate.")]
    NotAuthenticated,

    #[error("Invalid or expired token. Run 'tickrs init' to re-authenticate.")]
    Unauthorized,

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Rate limited. Please wait and try again.")]
    RateLimited,

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Failed to parse response: {0}")]
    ParseError(String),
}

impl TickTickClient {
    /// Create a new client with the stored token
    pub fn new() -> Result<Self> {
        let token = TokenStorage::load()?.ok_or(ApiError::NotAuthenticated)?;

        Self::with_token(token)
    }

    /// Create a new client with a specific token
    pub fn with_token(token: String) -> Result<Self> {
        Self::with_token_and_base_url(token, API_BASE_URL.to_string())
    }

    /// Create a new client with a specific token and base URL
    /// Primarily used for testing with mock servers
    pub fn with_token_and_base_url(token: String, base_url: String) -> Result<Self> {
        let client = Client::builder()
            .user_agent(format!("tickrs/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .context("Failed to create HTTP client")?;

        // Generate a device ID similar to what TickTickPy uses
        // Format: {"platform":"web","os":"OS X","device":"Firefox 95.0","name":"unofficial api!","version":4531,"id":"6490<hex>","channel":"website","campaign":"","websocket":""}
        let device_id = format!(
            r#"{{"platform":"web","os":"OS X","device":"Firefox 95.0","name":"unofficial api!","version":4531,"id":"6490{:x}","channel":"website","campaign":"","websocket":""}}"#,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
                as u64
        );

        Ok(Self {
            client,
            token,
            base_url,
            session_cookies: None,
            x_device: device_id,
        })
    }

    /// Login to the v2 API using username/password and return session cookies
    pub async fn login_v2(&mut self, username: &str, password: &str) -> Result<(), ApiError> {
        debug!("Logging in to v2 API as {}", username);

        #[derive(Debug, serde::Serialize)]
        struct LoginRequest {
            username: String,
            password: String,
        }

        #[derive(Debug, serde::Deserialize)]
        struct LoginResponse {
            token: String,
        }

        let request = LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        };

        let response = self
            .client
            .post(self.url_v2("/user/signon?wc=true&remember=true"))
            .json(&request)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:95.0) Gecko/20100101 Firefox/95.0")
            .header("X-Csrftoken", "")
            .header("X-Device", &self.x_device)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Referer", "https://ticktick.com/")
            .send()
            .await
            .map_err(|e| ApiError::NetworkError(e))?;

        let status = response.status();
        if status != 200 {
            let text = response.text().await.unwrap_or_default();
            return Err(ApiError::ServerError(format!(
                "Login failed with status {}: {}",
                status, text
            )));
        }

        // Extract token from response
        let login_response: LoginResponse = response.json().await
            .map_err(|e| ApiError::ParseError(format!("Failed to parse login response: {}", e)))?;

        // Update the stored token
        self.token = login_response.token.clone();

        // Set session cookie
        self.session_cookies = Some(format!("t={}", login_response.token));

        debug!("Successfully logged in to v2 API");
        Ok(())
    }

    /// Set the v2 session token directly (for when login is unavailable)
    pub fn set_v2_token(&mut self, token: &str) {
        self.token = token.to_string();
        self.session_cookies = Some(format!("t={}", token));
        debug!("Set v2 session token");
    }

    /// Get the full inbox project ID (e.g., "inbox127635041" instead of just "inbox")
    pub async fn get_inbox_id(&self) -> Result<String, ApiError> {
        #[derive(Debug, serde::Deserialize)]
        struct InboxResponse {
            id: String,
        }

        let response: InboxResponse = self.get_v2("/project/inbox").await?;
        Ok(response.id)
    }

    /// Build the full URL for an endpoint
    fn url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }

    /// Build the full URL for v2 API endpoint
    fn url_v2(&self, endpoint: &str) -> String {
        format!("{}{}", API_V2_BASE_URL, endpoint)
    }

    /// Make a GET request to the API
    #[instrument(skip(self), fields(endpoint = %endpoint))]
    pub async fn get<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T, ApiError> {
        debug!("GET {}", endpoint);

        let response = self
            .client
            .get(self.url(endpoint))
            .bearer_auth(&self.token)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Make a POST request to the API with JSON body
    #[instrument(skip(self, body), fields(endpoint = %endpoint))]
    pub async fn post<T: DeserializeOwned, B: serde::Serialize>(
        &self,
        endpoint: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        debug!("POST {}", endpoint);

        let response = self
            .client
            .post(self.url(endpoint))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Make a POST request without a body (for actions like complete)
    #[instrument(skip(self), fields(endpoint = %endpoint))]
    pub async fn post_empty<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T, ApiError> {
        debug!("POST {} (empty body)", endpoint);

        let response = self
            .client
            .post(self.url(endpoint))
            .bearer_auth(&self.token)
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Make a POST request to the v2 API with JSON body
    /// Make a POST request to the v2 API that returns empty body on success
    /// Uses Cookie-based authentication (not Bearer token)
    #[instrument(skip(self, body), fields(endpoint = %endpoint))]
    pub async fn post_v2_empty<B: serde::Serialize>(
        &self,
        endpoint: &str,
        body: &B,
    ) -> Result<(), ApiError> {
        debug!("POST v2 {} (expecting empty body)", endpoint);

        // Use session cookie for authentication (not Bearer token)
        let cookie = format!("t={}", self.token);
        
        let request = self
            .client
            .post(self.url_v2(endpoint))
            .header("X-Device", &self.x_device)
            .header("Cookie", cookie)
            .header("X-Csrftoken", "")
            .json(body);

        let response = request.send().await?;

        let status = response.status();
        
        match status {
            StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::UNAUTHORIZED => Err(ApiError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ApiError::NotFound(response.url().to_string())),
            StatusCode::BAD_REQUEST => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::BadRequest(text))
            }
            StatusCode::TOO_MANY_REQUESTS => Err(ApiError::RateLimited),
            _ if status.is_server_error() => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::ServerError(format!("{}: {}", status, text)))
            }
            _ => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::ServerError(format!(
                    "Unexpected status {}: {}",
                    status, text
                )))
            }
        }
    }

    /// Make a DELETE request to the API
    #[instrument(skip(self), fields(endpoint = %endpoint))]
    pub async fn delete(&self, endpoint: &str) -> Result<(), ApiError> {
        debug!("DELETE {}", endpoint);

        let response = self
            .client
            .delete(self.url(endpoint))
            .bearer_auth(&self.token)
            .send()
            .await?;

        self.handle_empty_response(response).await
    }

    /// Make a GET request to the v2 API
    #[instrument(skip(self), fields(endpoint = %endpoint))]
    pub async fn get_v2<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T, ApiError> {
        debug!("GET v2 {}", endpoint);

        // Use session cookie for authentication
        let cookie = format!("t={}", self.token);

        let response = self
            .client
            .get(self.url_v2(endpoint))
            .header("X-Device", &self.x_device)
            .header("Cookie", cookie)
            .header("X-Csrftoken", "")
            .send()
            .await?;

        self.handle_response(response).await
    }

    /// Handle API response and parse JSON
    async fn handle_response<T: DeserializeOwned>(
        &self,
        response: Response,
    ) -> Result<T, ApiError> {
        let status = response.status();
        let url = response.url().to_string();

        match status {
            StatusCode::OK | StatusCode::CREATED => {
                let text = response.text().await?;
                debug!("Response: {}", &text[..text.len().min(500)]);
                serde_json::from_str(&text).map_err(|e| {
                    ApiError::ParseError(format!("{}: {}", e, &text[..text.len().min(200)]))
                })
            }
            StatusCode::UNAUTHORIZED => Err(ApiError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ApiError::NotFound(url)),
            StatusCode::BAD_REQUEST => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::BadRequest(text))
            }
            StatusCode::TOO_MANY_REQUESTS => Err(ApiError::RateLimited),
            _ if status.is_server_error() => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::ServerError(format!("{}: {}", status, text)))
            }
            _ => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::ServerError(format!(
                    "Unexpected status {}: {}",
                    status, text
                )))
            }
        }
    }

    /// Handle API response for endpoints that return empty body
    async fn handle_empty_response(&self, response: Response) -> Result<(), ApiError> {
        let status = response.status();
        let url = response.url().to_string();

        match status {
            StatusCode::OK | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::UNAUTHORIZED => Err(ApiError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ApiError::NotFound(url)),
            StatusCode::BAD_REQUEST => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::BadRequest(text))
            }
            StatusCode::TOO_MANY_REQUESTS => Err(ApiError::RateLimited),
            _ if status.is_server_error() => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::ServerError(format!("{}: {}", status, text)))
            }
            _ => {
                let text = response.text().await.unwrap_or_default();
                Err(ApiError::ServerError(format!(
                    "Unexpected status {}: {}",
                    status, text
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_building() {
        // Create client with dummy token (won't make real requests)
        let client = TickTickClient::with_token("test_token".to_string()).unwrap();

        assert_eq!(
            client.url("/project"),
            "https://api.ticktick.com/open/v1/project"
        );
        assert_eq!(
            client.url("/project/123/task/456"),
            "https://api.ticktick.com/open/v1/project/123/task/456"
        );
    }

    #[test]
    fn test_api_error_display() {
        assert_eq!(
            ApiError::NotAuthenticated.to_string(),
            "Authentication required. Run 'tickrs init' to authenticate."
        );
        assert_eq!(
            ApiError::Unauthorized.to_string(),
            "Invalid or expired token. Run 'tickrs init' to re-authenticate."
        );
        assert_eq!(
            ApiError::NotFound("/project/123".to_string()).to_string(),
            "Resource not found: /project/123"
        );
    }
}
