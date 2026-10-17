// Simple AES-GCM encryption for localhost transmission
const PSK = "GD3_LOCAL_SECURE_KEY_2026_YOUTUBE"; // 32-byte string

export async function encryptCookies(cookieString: string): Promise<{ encrypted: string; iv: string } | null> {
  if (!cookieString) return null;
  try {
    const encoder = new TextEncoder();
    const keyData = encoder.encode(PSK.padEnd(32, '0').slice(0, 32));
    const key = await crypto.subtle.importKey(
      "raw",
      keyData,
      { name: "AES-GCM" },
      false,
      ["encrypt"]
    );

    const iv = crypto.getRandomValues(new Uint8Array(12));
    const encodedText = encoder.encode(cookieString);

    const encryptedBuffer = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv },
      key,
      encodedText
    );

    // Convert ArrayBuffer to Base64
    const encryptedArray = new Uint8Array(encryptedBuffer);
    const encryptedBase64 = btoa(String.fromCharCode(...encryptedArray));
    const ivBase64 = btoa(String.fromCharCode(...iv));

    return { encrypted: encryptedBase64, iv: ivBase64 };
  } catch (error) {
    console.error("[GD3 Crypto] Failed to encrypt cookies:", error);
    return null;
  }
}

export async function getDomainCookies(domain: string): Promise<string> {
  return new Promise((resolve) => {
    chrome.cookies.getAll({ domain }, (cookies) => {
      if (chrome.runtime.lastError) {
        console.warn("[GD3 Crypto] Failed to read domain cookies:", chrome.runtime.lastError.message);
        resolve("");
        return;
      }
      if (!cookies || cookies.length === 0) {
        resolve("");
        return;
      }
      resolve(cookiesToHeader(cookies));
    });
  });
}

export async function getUrlCookies(url: string): Promise<string> {
  if (!/^https?:\/\//i.test(url)) {
    return "";
  }
  return new Promise((resolve) => {
    chrome.cookies.getAll({ url }, (cookies) => {
      if (chrome.runtime.lastError) {
        console.warn("[GD3 Crypto] Failed to read URL cookies:", chrome.runtime.lastError.message);
        resolve("");
        return;
      }
      if (!cookies || cookies.length === 0) {
        resolve("");
        return;
      }
      resolve(cookiesToHeader(cookies));
    });
  });
}

function cookiesToHeader(cookies: chrome.cookies.Cookie[]): string {
  return cookies.map(c => `${c.name}=${c.value}`).join("; ");
}
