//! 设置页草稿：把原生控件文本/勾选状态转换为经过校验的配置。
use super::*;
use crate::config::{Config, InjectionMode, SecretStorage, SecretString, StorageMode};

pub(super) fn field_value(
    current: &toml::Value,
    field: &str,
    input: &str,
    checked: bool,
) -> Result<toml::Value, &'static str> {
    Ok(match current {
        toml::Value::Boolean(_) => toml::Value::Boolean(checked),
        toml::Value::Integer(_) => toml::Value::Integer(
            input
                .trim()
                .parse::<u32>()
                .map(i64::from)
                .map_err(|_| "请输入 0–4294967295 范围内的整数。")?,
        ),
        toml::Value::Array(_) => {
            let parsed: toml::Table = format!("value = {input}")
                .parse()
                .map_err(|_| "请输入 TOML 数组，例如 [443] 或 [\"127.0.0.1\"]。")?;
            if parsed.len() != 1 {
                return Err("只允许一个 TOML 数组，不能附加其他字段。");
            }
            let value = parsed
                .get("value")
                .filter(|v| v.is_array())
                .ok_or("请输入 TOML 数组。")?
                .clone();
            let valid = match field {
                "whitelist" => value.clone().try_into::<Vec<String>>().is_ok_and(|items| {
                    items.iter().all(|item| {
                        item.parse::<ipnet::IpNet>().is_ok()
                            || item.parse::<std::net::IpAddr>().is_ok()
                    })
                }),
                "connect_allowed_ports" => value.clone().try_into::<Vec<u16>>().is_ok(),
                "allowed_origins" => value.clone().try_into::<Vec<String>>().is_ok(),
                _ => false,
            };
            if !valid {
                return Err("数组元素无效；请检查 IP/CIDR、端口范围或带引号的 Origin。");
            }
            value
        }
        _ => toml::Value::String(input.to_owned()),
    })
}

impl Settings {
    pub(super) fn draft(&self) -> Result<Config, String> {
        let mut config = self.config.clone().ok_or("配置未加载，不能操作。")?;
        config.ui.theme = self.theme_mode();
        config.upstream.url = text(self.url).trim().to_owned();
        config.listen.address = text(self.ip)
            .trim()
            .parse()
            .map_err(|_| "监听 IP 无效，请输入 IPv4 或 IPv6 地址。")?;
        config.listen.port = text(self.port)
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or("监听端口必须在 1–65535 之间。")?;
        config.keys.upstream_api_key = SecretString::new(text(self.key));
        config.keys.injection_mode = match selected(self.injection) {
            1 => InjectionMode::Always,
            2 => InjectionMode::Missing,
            _ => InjectionMode::Off,
        };
        config.storage.mode = if selected(self.storage) == 1 {
            StorageMode::Portable
        } else {
            StorageMode::Standard
        };
        config.storage.secrets = if selected(self.secrets) == 1 {
            SecretStorage::Plaintext
        } else {
            SecretStorage::Dpapi
        };
        // SAFETY: Live checkbox with no pointer arguments.
        config.proxy.auto_start = unsafe { SendMessageW(self.auto_start, BM_GETCHECK, 0, 0) != 0 };
        let mut value = toml::Value::try_from(&config).map_err(|_| "无法准备设置草稿。")?;
        for &(control, section, field) in &self.fields {
            let target = value
                .get_mut(section)
                .and_then(|v| v.get_mut(field))
                .ok_or("未知设置字段。")?;
            // SAFETY: Live native child; only checkboxes interpret BM_GETCHECK.
            let checked = unsafe { SendMessageW(control, BM_GETCHECK, 0, 0) != 0 };
            *target = field_value(target, field, &text(control), checked)
                .map_err(|error| format!("{section}.{field}：{error}"))?;
        }
        config = value
            .try_into()
            .map_err(|_| "设置类型或范围无效；请检查当前及其他分类的输入。")?;
        config
            .validate_for_save()
            .map_err(|e| format!("配置校验失败（字段 {}），请检查输入及注入模式。", e.field))?;
        Ok(config)
    }
}
