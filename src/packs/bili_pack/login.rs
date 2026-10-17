use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct QrCodeInfo {
    pub url: String, // 实际用于生成二维码的字符串
    pub qrcode_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PollResult {
    pub code: i32, // 0=成功, 86101=未扫码, 86090=已扫码待确认, 86038=已过期
    pub message: String,
    pub cookies: Option<String>,
}

/// 步骤一：向 B 站请求一个新的二维码和 qrcode_key
pub async fn generate_qrcode() -> Result<QrCodeInfo> {
    let client = Client::new();
    let res: Value = client
        .get("https://passport.bilibili.com/x/passport-login/web/qrcode/generate")
        .send()
        .await?
        .json()
        .await?;

    let code = res["code"].as_i64().unwrap_or(-1);
    if code != 0 {
        return Err(anyhow!(
            "Failed to generate Bilibili QR code: {}",
            res["message"].as_str().unwrap_or("")
        ));
    }

    let data = &res["data"];
    Ok(QrCodeInfo {
        url: data["url"].as_str().unwrap_or("").to_string(),
        qrcode_key: data["qrcode_key"].as_str().unwrap_or("").to_string(),
    })
}

/// 步骤二：轮询扫码结果（前端每秒调用一次该接口或由后台循环推送）
pub async fn poll_qrcode(qrcode_key: &str) -> Result<PollResult> {
    let client = Client::new();
    let url = format!(
        "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}",
        qrcode_key
    );

    let response = client.get(&url).send().await?;

    // 如果扫码成功，B站会把登录态(SESSDATA, bili_jct 等)写进 Set-Cookie
    let mut extracted_cookies = Vec::new();
    for (k, v) in response.headers() {
        if k.as_str().to_lowercase() == "set-cookie"
            && let Ok(cookie_str) = v.to_str()
        {
            // 将分号前的 KV 键值对提取出来
            let cookie_part = cookie_str.split(';').next().unwrap_or("");
            extracted_cookies.push(cookie_part.to_string());
        }
    }

    let res: Value = response.json().await?;
    let data = &res["data"];
    let status_code = data["code"].as_i64().unwrap_or(-1) as i32;
    let message = data["message"].as_str().unwrap_or("").to_string();

    // 只有 code == 0 才是最终确认登录成功
    let final_cookie = if status_code == 0 && !extracted_cookies.is_empty() {
        Some(extracted_cookies.join("; "))
    } else {
        None
    };

    Ok(PollResult {
        code: status_code,
        message,
        cookies: final_cookie,
    })
}
