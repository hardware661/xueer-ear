//! 豆包的「虚拟手机」：匿名注册一台安卓设备拿设备号，存在 `%LOCALAPPDATA%\XueerEar\doubao-device.json`（不进仓）。
//! 没有账号参与；服务端不认这台设备了（会话失败且一个字没给），删掉文件，下次重新注册一台。
use crate::{Result, https::Request};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 豆包输入法安卓客户端的应用身份（公开的客户端常量）
pub const AID: &str = "401734";
const APP_NAME: &str = "oime";
const VERSION_CODE: &str = "100406010";
const VERSION_NAME: &str = "1.4.6";
const PACKAGE: &str = "com.bytedance.android.doubaoime";
/// 注册接口：新域名在前，老域名兜底
pub const REGISTER_HOSTS: &[&str] = &["log-klink.zijieapi.com", "log.snssdk.com"];
const REGISTER_PATH: &str = "/service/2/device_register/";
const MAX_FILE_BYTES: u64 = 4096;

/// 几款常见的真机（品牌、型号、安卓版本、API、系统版本号、分辨率、dpi），注册时随机挑一台
const PHONES: &[[&str; 7]] = &[
    [
        "google",
        "Pixel 7 Pro",
        "16",
        "36",
        "BP2A.250605.031.A2",
        "1080*2340",
        "420",
    ],
    [
        "google",
        "Pixel 8",
        "14",
        "34",
        "AP2A.240805.005",
        "1080*2400",
        "420",
    ],
    [
        "Xiaomi",
        "23127PN0CC",
        "14",
        "34",
        "UKQ1.230804.001",
        "1200*2670",
        "440",
    ],
    [
        "Redmi",
        "23113RKC6C",
        "14",
        "34",
        "UKQ1.231003.002",
        "1440*3200",
        "560",
    ],
    [
        "vivo",
        "V2309A",
        "14",
        "34",
        "UP1A.231005.007",
        "1260*2800",
        "480",
    ],
    [
        "OPPO",
        "PHZ110",
        "14",
        "34",
        "UKQ1.231108.001",
        "1264*2780",
        "480",
    ],
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phone {
    pub brand: String,
    pub model: String,
    pub os: String,
    pub api: String,
    pub build: String,
    pub resolution: String,
    pub dpi: String,
}

impl Phone {
    pub(crate) fn pick(seed: usize) -> Self {
        let [brand, model, os, api, build, resolution, dpi] =
            PHONES[seed % PHONES.len()].map(String::from);
        Self {
            brand,
            model,
            os,
            api,
            build,
            resolution,
            dpi,
        }
    }

    pub fn user_agent(&self) -> String {
        format!(
            "com.bytedance.android.doubaoime/{VERSION_CODE} (Linux; U; Android {}; en_US; {}; Build/{}; Cronet/TTNetVersion:94cf429a 2025-11-17 QuicVersion:1f89f732 2025-05-08)",
            self.os, self.model, self.build
        )
    }

    /// 应用和设备参数：注册时既放在网址里，也放进请求体的 header
    fn params(&self) -> Vec<(&'static str, String)> {
        let mut params: Vec<(&'static str, String)> = [
            ("aid", AID),
            ("app_name", APP_NAME),
            ("channel", "official"),
            ("version_code", VERSION_CODE),
            ("version_name", VERSION_NAME),
            ("manifest_version_code", VERSION_CODE),
            ("update_version_code", VERSION_CODE),
            ("package", PACKAGE),
            ("device_platform", "android"),
            ("os", "android"),
        ]
        .map(|(k, v)| (k, v.to_string()))
        .into();
        params.extend([
            ("os_api", self.api.clone()),
            ("os_version", self.os.clone()),
            ("device_type", self.model.clone()),
            ("device_brand", self.brand.clone()),
            ("device_model", self.model.clone()),
            ("resolution", self.resolution.clone()),
            ("dpi", self.dpi.clone()),
            ("language", "zh".into()),
            ("timezone", "8".into()),
            ("access", "wifi".into()),
            ("rom", self.build.clone()),
            ("rom_version", self.build.clone()),
        ]);
        params
    }
}

/// 一台注册好的虚拟手机
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub did: String,
    pub phone: Phone,
}

/// 注册时随机生成的几个标识
pub struct Identity {
    pub cdid: String,
    pub openudid: String,
    pub clientudid: String,
    pub now_ms: u64,
    pub seed: usize,
}

impl Identity {
    pub fn random(now_ms: u64) -> Result<Self> {
        let mut bytes = [0_u8; 48];
        getrandom::fill(&mut bytes).map_err(|_| "random generation failed")?;
        Ok(Self {
            cdid: uuid(&bytes[..16]),
            openudid: hex(&bytes[16..24]),
            clientudid: uuid(&bytes[24..40]),
            now_ms,
            seed: usize::from(bytes[40]),
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 16 个随机字节排成 UUID v4 的样子
pub(crate) fn uuid(bytes: &[u8]) -> String {
    let mut b = [0_u8; 16];
    b.copy_from_slice(&bytes[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// 网址查询参数的值：字母数字和 -_.~ 原样，其余按字节 %XX
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 注册请求：网址带应用和设备参数，请求体是同样的参数加上随机标识
pub fn registration(host: &'static str, identity: &Identity) -> Request {
    let phone = Phone::pick(identity.seed);
    let mut query = phone.params();
    let mut header = serde_json::Map::new();
    for (k, v) in &query {
        header.insert((*k).into(), v.clone().into());
    }
    for (k, v) in [
        ("device_id", serde_json::json!(0)),
        ("install_id", serde_json::json!(0)),
        ("cdid", identity.cdid.clone().into()),
        ("openudid", identity.openudid.clone().into()),
        ("clientudid", identity.clientudid.clone().into()),
        ("region", "CN".into()),
        ("tz_name", "Asia/Shanghai".into()),
        ("tz_offset", serde_json::json!(28800)),
        ("sim_region", "cn".into()),
        ("carrier_region", "cn".into()),
        ("cpu_abi", "arm64-v8a".into()),
        ("build_serial", "unknown".into()),
        ("not_request_sender", serde_json::json!(0)),
    ] {
        header.insert(k.into(), v);
    }
    let body = serde_json::json!({
        "magic_tag": "ss_app_log",
        "header": header,
        "_gen_time": identity.now_ms,
    });
    query.extend([
        ("ssmix", "a".into()),
        ("_rticket", identity.now_ms.to_string()),
        ("cdid", identity.cdid.clone()),
        ("ac", "wifi".into()),
    ]);
    let query: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect();
    Request {
        host,
        path: format!("{REGISTER_PATH}?{}", query.join("&")),
        headers: vec![
            ("User-Agent".into(), phone.user_agent()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: body.to_string(),
    }
}

/// 从注册回包里取设备号；拿不到（或是 0）就是被拒了
pub fn parse_registration(status: u32, body: &[u8]) -> Result<String> {
    if status != 200 {
        return Err(format!("豆包注册设备失败：HTTP {status}").into());
    }
    let value: serde_json::Value = serde_json::from_slice(body)?;
    let did = value["device_id_str"]
        .as_str()
        .map(String::from)
        .or_else(|| value["device_id"].as_u64().map(|n| n.to_string()))
        .unwrap_or_default();
    if did.is_empty() || did == "0" || !did.bytes().all(|b| b.is_ascii_digit()) {
        return Err("豆包没给设备号".into());
    }
    Ok(did)
}

impl Device {
    /// 设备文件：测试可以用 DOUBAO_DEVICE_FILE 指到别处，不碰主人的那台
    pub fn path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("DOUBAO_DEVICE_FILE") {
            return Ok(path.into());
        }
        Ok(
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 不可用")?)
                .join("XueerEar/doubao-device.json"),
        )
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_FILE_BYTES => Err("豆包设备文件太大".into()),
            Ok(_) => {
                let device: Self = serde_json::from_slice(&std::fs::read(path)?)?;
                Ok((!device.did.is_empty()).then_some(device))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec(self)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }

    /// 服务端不认这台了：删掉，下次重新注册
    pub fn forget() {
        if let Ok(path) = Self::path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// 用上次那台；没有就注册一台并存下来
    #[cfg(windows)]
    pub fn load_or_register(now_ms: u64) -> Result<Self> {
        let path = Self::path()?;
        if let Some(device) = Self::load(&path)? {
            return Ok(device);
        }
        let identity = Identity::random(now_ms)?;
        let mut last = None;
        for host in REGISTER_HOSTS {
            let request = registration(host, &identity);
            let result = crate::https::send(&request)
                .map_err(Into::into)
                .and_then(|(status, body)| parse_registration(status, &body));
            match result {
                Ok(did) => {
                    let device = Self {
                        did,
                        phone: Phone::pick(identity.seed),
                    };
                    device.save(&path)?;
                    return Ok(device);
                }
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| "豆包注册设备失败".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity {
            cdid: "c".into(),
            openudid: "0011223344556677".into(),
            clientudid: "u".into(),
            now_ms: 1_700_000_000_000,
            seed: 0,
        }
    }

    #[test]
    fn registration_puts_app_and_phone_params_in_url_and_body() {
        let request = registration("log-klink.zijieapi.com", &identity());
        assert_eq!(request.host, "log-klink.zijieapi.com");
        assert!(
            request
                .path
                .starts_with("/service/2/device_register/?aid=401734&")
        );
        assert!(request.path.contains("device_model=Pixel%207%20Pro"));
        assert!(request.path.contains("resolution=1080%2A2340"));
        assert!(request.path.contains("_rticket=1700000000000"));
        let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body["magic_tag"], "ss_app_log");
        assert_eq!(body["header"]["aid"], "401734");
        assert_eq!(body["header"]["openudid"], "0011223344556677");
        assert_eq!(body["header"]["device_id"], 0);
        assert!(
            request
                .headers
                .iter()
                .any(|(k, v)| k == "User-Agent" && v.contains("Pixel 7 Pro"))
        );
    }

    #[test]
    fn device_numbers_come_from_either_field_and_zero_means_refused() {
        assert_eq!(
            parse_registration(200, br#"{"device_id_str":"123456"}"#).unwrap(),
            "123456"
        );
        assert_eq!(
            parse_registration(200, br#"{"device_id":987}"#).unwrap(),
            "987"
        );
        assert!(parse_registration(200, br#"{"device_id":0}"#).is_err());
        assert!(parse_registration(200, b"{}").is_err());
        assert!(parse_registration(403, br#"{"device_id":1}"#).is_err());
    }

    #[test]
    fn random_identities_look_like_the_official_client() {
        let a = Identity::random(1).unwrap();
        assert_eq!(a.cdid.len(), 36);
        assert_eq!(&a.cdid[14..15], "4");
        assert_eq!(a.openudid.len(), 16);
        assert_ne!(a.cdid, Identity::random(1).unwrap().cdid);
    }

    #[test]
    fn saved_devices_load_back_and_missing_files_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doubao-device.json");
        assert_eq!(Device::load(&path).unwrap(), None);
        let device = Device {
            did: "42".into(),
            phone: Phone::pick(1),
        };
        device.save(&path).unwrap();
        assert_eq!(Device::load(&path).unwrap(), Some(device));
    }

    #[test]
    fn user_agent_names_the_phone() {
        let ua = Phone::pick(2).user_agent();
        assert!(ua.starts_with("com.bytedance.android.doubaoime/100406010 (Linux; U; Android 14;"));
        assert!(ua.contains("23127PN0CC"));
    }
}
