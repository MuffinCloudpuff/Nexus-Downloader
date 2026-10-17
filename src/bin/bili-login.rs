use gd3_daemon::services::bilibili_cookie_store;
use qrcode::QrCode;
use qrcode::render::unicode;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();

    // 1. 获取二维码
    let qr_req_url = "https://passport.bilibili.com/x/passport-login/web/qrcode/generate";
    let qr_resp: Value = client
        .get(qr_req_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send()
        .await?
        .json()
        .await?;

    if qr_resp["code"].as_i64() != Some(0) {
        println!("获取二维码失败: {}", qr_resp["message"]);
        return Ok(());
    }

    let qr_url = qr_resp["data"]["url"].as_str().unwrap();
    let qrcode_key = qr_resp["data"]["qrcode_key"].as_str().unwrap();

    // 2. 渲染到终端
    let code = QrCode::new(qr_url)?;
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Dark)
        .light_color(unicode::Dense1x2::Light)
        .build();

    println!("\n==============================================");
    println!("        Nexus Downloader - Bilibili 登录       ");
    println!("==============================================\n");
    println!("请使用 Bilibili 手机 App 扫描以下二维码登录：\n");
    println!("{}", image);
    println!("\n等待扫码...\n");

    // 3. 轮询状态
    let poll_url = format!(
        "https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}",
        qrcode_key
    );
    loop {
        sleep(Duration::from_secs(2)).await;

        let resp = client
            .get(&poll_url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .send()
            .await?;

        let mut cookies = Vec::new();
        for cookie in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(c) = cookie.to_str() {
                cookies.push(c.to_string());
            }
        }

        let poll_data: Value = resp.json().await?;
        let code = poll_data["data"]["code"].as_i64().unwrap_or(-1);

        match code {
            0 => {
                println!("✅ 登录成功！");
                let mut cookie_parts = Vec::new();
                for c in &cookies {
                    if let Some(key_val) = c.split(';').next() {
                        cookie_parts.push(key_val.to_string());
                    }
                }

                let final_cookie = cookie_parts.join("; ");

                if final_cookie.is_empty() {
                    println!("未能提取到 Cookie，登录可能异常。");
                } else {
                    bilibili_cookie_store::save_bilibili_cookie(&final_cookie)?;
                    println!("🎉 大会员身份已成功保存！");
                    println!("凭据已加密保存到本地。");
                    println!("你可以关闭此窗口，下次在浏览器点击下载时将自动以最高画质进行！");
                }
                break;
            }
            86101 => {
                // Not scanned
            }
            86090 => {
                println!("❌ 二维码已过期，请重新运行本程序。");
                break;
            }
            86038 => {
                println!("📲 已扫码，请在手机端确认登录...");
            }
            _ => {
                println!("未知状态码: {}", code);
            }
        }
    }

    Ok(())
}
