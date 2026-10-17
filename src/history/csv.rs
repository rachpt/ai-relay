use super::*;
use std::io::Write;

pub(super) fn needs_formula_guard(value: &str) -> bool {
    // Spreadsheet importers can ignore leading whitespace/control characters.
    value.starts_with(['\t', '\r', '\n'])
        || value
            .trim_start_matches(|c: char| c.is_whitespace() || c.is_control() || c == '\u{feff}')
            .starts_with(['=', '+', '-', '@'])
}
fn cell(writer: &mut impl Write, value: &str) -> std::io::Result<()> {
    writer.write_all(b"\"")?;
    if needs_formula_guard(value) {
        writer.write_all(b"'")?;
    }
    let mut rest = value;
    while let Some(index) = rest.find('"') {
        writer.write_all(&rest.as_bytes()[..index])?;
        writer.write_all(b"\"\"")?;
        rest = &rest[index + 1..];
    }
    writer.write_all(rest.as_bytes())?;
    writer.write_all(b"\"")
}
fn line<'a>(writer: &mut impl Write, values: impl IntoIterator<Item = &'a str>) -> Result<()> {
    for (index, value) in values.into_iter().enumerate() {
        if index != 0 {
            writer.write_all(b",")?;
        }
        cell(writer, value)?;
    }
    writer.write_all(b"\r\n")?;
    Ok(())
}
pub(super) fn header(writer: &mut impl Write) -> Result<()> {
    line(
        writer,
        [
            "request_id",
            "started_utc_ms",
            "started_time",
            "time_zone",
            "requested_model",
            "response_model",
            "provider_origin",
            "protocol",
            "interface",
            "status",
            "outcome",
            "latency_ms",
            "total_ms",
            "usage_provider",
            "observation_format",
            "completeness",
            "usage_complete",
            "usage_partial",
            "usage_unknown",
            "truncated",
            "parse_error",
            "parse_limit_exceeded",
            "provider_error",
            "arithmetic_overflow",
            "unsupported",
            "input_tokens",
            "output_tokens",
            "total_tokens",
            "cached_input_tokens",
            "reasoning_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
            "input_coverage",
            "output_coverage",
            "total_coverage",
            "cached_input_coverage",
            "reasoning_coverage",
            "cache_creation_input_coverage",
            "cache_read_input_coverage",
            "client_alias",
            "upstream_alias",
        ],
    )
}
pub(super) fn record(
    writer: &mut impl Write,
    record: &RequestRecord,
    zone: TimeZone,
    client_alias: Option<&CredentialAlias>,
    upstream_alias: Option<&CredentialAlias>,
) -> Result<()> {
    let instant = chrono::DateTime::from_timestamp_millis(record.started_ms)
        .ok_or(Error::Invalid("export timestamp"))?;
    let time = match zone {
        TimeZone::Utc => instant.to_rfc3339(),
        TimeZone::Local => instant.with_timezone(&chrono::Local).to_rfc3339(),
    };
    let completeness = record.report.completeness();
    let complete = completeness == ai_relay_usage::UsageCompleteness::Complete;
    let partial = completeness == ai_relay_usage::UsageCompleteness::Partial;
    let unknown = completeness == ai_relay_usage::UsageCompleteness::Unknown;
    let unsupported = matches!(
        record.report.format,
        ai_relay_usage::ObservationFormat::UnsupportedContentType
            | ai_relay_usage::ObservationFormat::UnsupportedContentEncoding
    );
    let mut fields = vec![
        record.request_id.as_str().to_owned(),
        record.started_ms.to_string(),
        time,
        match zone {
            TimeZone::Utc => "UTC",
            TimeZone::Local => "Local",
        }
        .into(),
        record
            .requested_model
            .as_ref()
            .map(|v| v.as_str())
            .unwrap_or_default()
            .into(),
        record.report.model.clone().unwrap_or_default(),
        record
            .provider
            .as_ref()
            .map(|v| v.as_str())
            .unwrap_or_default()
            .into(),
        record.protocol.text().into(),
        record.interface.text().into(),
        record.status.map(|v| v.to_string()).unwrap_or_default(),
        record.outcome.text().into(),
        record.latency_ms.to_string(),
        record.total_ms.to_string(),
        provider_text(record.report.provider).into(),
        format_text(record.report.format).into(),
        if complete {
            "complete"
        } else if partial {
            "partial"
        } else {
            "unknown"
        }
        .into(),
    ];
    for flag in [
        complete,
        partial,
        unknown,
        record.report.truncated,
        record.report.parse_error,
        record.report.limit_exceeded,
        record.report.provider_error,
        record.report.arithmetic_overflow,
        unsupported,
    ] {
        fields.push(u8::from(flag).to_string());
    }
    let tokens = usage_values(&record.report.usage);
    for token in tokens {
        fields.push(token.map(|v| v.to_string()).unwrap_or_default());
    }
    for token in tokens {
        fields.push(u8::from(token.is_some()).to_string());
    }
    fields.push(
        client_alias
            .map(CredentialAlias::as_str)
            .unwrap_or_default()
            .into(),
    );
    fields.push(
        upstream_alias
            .map(CredentialAlias::as_str)
            .unwrap_or_default()
            .into(),
    );
    line(writer, fields.iter().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guards_formula_prefixes_after_whitespace_and_escapes_quotes() {
        let mut bytes = Vec::new();
        line(
            &mut bytes,
            [" =SUM(1,2)", "\n@command", "\"quoted\"", "普通别名"],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "\"' =SUM(1,2)\",\"'\n@command\",\"\"\"quoted\"\"\",\"普通别名\"\r\n"
        );
    }
    #[test]
    fn writer_errors_are_propagated() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::WriteZero))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(matches!(header(&mut Broken), Err(Error::Io(_))));
    }
}
