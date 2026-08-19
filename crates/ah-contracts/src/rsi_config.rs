//! rsi 配置模型(对齐 openjiuwen/rsi/config/config.py)。
//!
//! 纯数据契约:12 个配置类 + 类型解析辅助函数(from_dict 语义)。

/// 从 YAML/JSON 值解析 int(对齐 _int_value;bool 拒绝)。
pub fn parse_int(value: Option<&serde_json::Value>, default: i64) -> Result<i64, String> {
    match value {
        None => Ok(default),
        Some(v) => {
            if v.is_boolean() {
                return Err(format!("expected an integer value, got {v}"));
            }
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| format!("expected an integer value, got {v}"))
        }
    }
}

/// 从 YAML/JSON 值解析 float(对齐 _float_value;bool 拒绝)。
pub fn parse_float(value: Option<&serde_json::Value>, default: f64) -> Result<f64, String> {
    match value {
        None => Ok(default),
        Some(v) => {
            if v.is_boolean() {
                return Err(format!("expected a numeric value, got {v}"));
            }
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                .ok_or_else(|| format!("expected a numeric value, got {v}"))
        }
    }
}

/// 从 YAML/JSON 值解析 bool(对齐 _bool_value;仅识别的真/假值)。
pub fn parse_bool(value: Option<&serde_json::Value>, default: bool) -> Result<bool, String> {
    match value {
        None => Ok(default),
        Some(v) => {
            if let Some(b) = v.as_bool() {
                return Ok(b);
            }
            if let Some(n) = v.as_i64() {
                if n == 0 {
                    return Ok(false);
                }
                if n == 1 {
                    return Ok(true);
                }
            }
            if let Some(s) = v.as_str() {
                let norm = s.trim().to_lowercase();
                match norm.as_str() {
                    "true" | "yes" | "y" | "1" | "on" => return Ok(true),
                    "false" | "no" | "n" | "0" | "off" => return Ok(false),
                    _ => {}
                }
            }
            Err(format!("expected a boolean value, got {v}"))
        }
    }
}

/// 从 YAML/JSON 值解析字符串列表(对齐 _string_list;标量→单元素列表)。
pub fn parse_string_list(value: Option<&serde_json::Value>) -> Result<Vec<String>, String> {
    match value {
        None => Ok(Vec::new()),
        Some(v) => {
            if let Some(s) = v.as_str() {
                return Ok(vec![s.to_string()]);
            }
            if let Some(arr) = v.as_array() {
                return Ok(arr
                    .iter()
                    .filter(|x| !x.is_null())
                    .map(|x| x.to_string())
                    .collect());
            }
            Err(format!("expected a list of strings, got {v}"))
        }
    }
}
