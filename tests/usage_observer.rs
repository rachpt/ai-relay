// 直接编译独立模块，无需修改生产 lib 的导出。
#[allow(dead_code, unused_imports)]
#[path = "../src/usage/mod.rs"]
mod usage;

use usage::*;

fn observe(body: &[u8], sse: bool, chunk: usize, complete: bool) -> UsageReport {
    let mut observer = UsageObserver::new(
        if sse {
            "text/event-stream; charset=utf-8"
        } else {
            "application/json"
        },
        None,
    );
    for bytes in body.chunks(chunk) {
        observer.feed(bytes);
    }
    observer.finish(complete)
}

#[test]
fn chat_json_details_are_subsets() {
    let report = observe(br#"{"model":"gpt-test","usage":{"prompt_tokens":100,"completion_tokens":20,"total_tokens":120,"prompt_tokens_details":{"cached_tokens":80},"completion_tokens_details":{"reasoning_tokens":10}}}"#, false, 1, true);
    assert_eq!(report.provider, Provider::OpenAiChat);
    assert_eq!(report.completeness(), UsageCompleteness::Complete);
    assert_eq!(report.usage.total_tokens, Some(120));
    assert_eq!(report.usage.cached_input_tokens, Some(80));
    assert_eq!(report.usage.reasoning_tokens, Some(10));
    assert_eq!(report.model.as_deref(), Some("gpt-test"));
}

#[test]
fn sse_every_split_utf8_crlf_multidata_and_duplicate_final() {
    let body = concat!(
        ": 心跳\r\n\r\n",
        "data: {\"object\":\"chat.completion.chunk\",\"model\":\"m\",\r\n",
        "data: \"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\r\n\r\n",
        "data: {\"object\":\"chat.completion.chunk\",\"usage\":{\"total_tokens\":999}}\n\n",
        "data: [DONE]\n\n",
        "not json"
    );
    for split in 0..=body.len() {
        let mut o = UsageObserver::new("TEXT/EVENT-STREAM", Some("identity"));
        o.feed(&body.as_bytes()[..split]);
        o.feed(&body.as_bytes()[split..]);
        let r = o.finish(true);
        assert_eq!(r.usage.total_tokens, Some(5), "split {split}");
        assert!(!r.parse_error && !r.truncated);
    }
    assert_eq!(
        observe(body.as_bytes(), true, 1, true).usage.total_tokens,
        Some(5)
    );
}

#[test]
fn responses_json_and_completed_event() {
    let response = r#"{"object":"response","model":"test/response","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15,"input_tokens_details":{"cached_tokens":4},"output_tokens_details":{"reasoning_tokens":2}}}"#;
    let json = observe(response.as_bytes(), false, 3, true);
    let stream = format!(
        "event: response.completed\ndata: {{\"response\":{response}}}\n\nevent: response.completed\ndata: {{\"response\":{response}}}\n\n"
    );
    let sse = observe(stream.as_bytes(), true, 1, true);
    assert_eq!(json.usage, sse.usage);
    assert_eq!(sse.provider, Provider::OpenAiResponses);
    assert_eq!(sse.usage.total_tokens, Some(15));
    assert_eq!(sse.usage.reasoning_tokens, Some(2));
}

#[test]
fn responses_failed_and_incomplete_preserve_usage_and_freeze_terminal_snapshot() {
    for kind in ["response.failed", "response.incomplete"] {
        let terminal = format!(
            "event: {kind}\ndata: {{\"type\":\"{kind}\",\"response\":{{\"object\":\"response\",\"model\":\"test/response\",\"error\":{{\"message\":\"private-error-payload\"}},\"incomplete_details\":{{\"reason\":\"max_output_tokens\"}},\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\"total_tokens\":15,\"input_tokens_details\":{{\"cached_tokens\":4}},\"output_tokens_details\":{{\"reasoning_tokens\":2}}}}}}}}\n\n"
        );
        let duplicate = terminal
            .replace("test/response", "duplicate-model")
            .replace(":15", ":999");
        let stream = format!(
            "{terminal}{duplicate}event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"model\":\"late-model\",\"usage\":{{\"total_tokens\":1000}}}}}}\n\n"
        );
        for complete in [true, false] {
            for chunk in [1, 7, stream.len()] {
                let r = observe(stream.as_bytes(), true, chunk, complete);
                assert_eq!(r.provider, Provider::OpenAiResponses);
                assert_eq!(r.model.as_deref(), Some("test/response"));
                assert_eq!(r.usage.input_tokens, Some(10));
                assert_eq!(r.usage.output_tokens, Some(5));
                assert_eq!(r.usage.total_tokens, Some(15));
                assert_eq!(r.usage.cached_input_tokens, Some(4));
                assert_eq!(r.usage.reasoning_tokens, Some(2));
                assert_eq!(r.completeness(), UsageCompleteness::Complete);
                assert!(r.provider_error);
                assert_eq!(r.truncated, !complete);
                assert!(!r.parse_error);
                assert!(!format!("{r:?}").contains("private-error-payload"));
            }
        }
    }
}

#[test]
fn responses_terminal_without_usage_is_still_terminal() {
    for kind in [
        "response.completed",
        "response.failed",
        "response.incomplete",
    ] {
        let stream = format!(
            "event: {kind}\ndata: {{\"response\":{{\"model\":\"first-model\"}}}}\n\nevent: response.completed\ndata: {{\"response\":{{\"model\":\"late-model\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":2,\"total_tokens\":3}}}}}}\n\n"
        );
        let r = observe(stream.as_bytes(), true, 1, true);
        assert_eq!(r.provider, Provider::OpenAiResponses);
        assert_eq!(r.model.as_deref(), Some("first-model"));
        assert_eq!(r.usage, TokenUsage::default());
        assert_eq!(r.provider_error, kind != "response.completed");
        assert!(!r.truncated && !r.parse_error);
    }
}

#[test]
fn anthropic_snapshots_not_deltas() {
    let sse = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-test\",\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":2,\"cache_read_input_tokens\":3,\"output_tokens\":1}}}\n\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":100}}\n\n"
    );
    let r = observe(sse.as_bytes(), true, 1, true);
    assert_eq!(r.usage.output_tokens, Some(7));
    assert_eq!(r.usage.total_tokens, Some(22));
    assert_eq!(r.completeness(), UsageCompleteness::Complete);
}

#[test]
fn anthropic_missing_cache_is_not_complete_or_zero() {
    let r = observe(
        br#"{"type":"message","usage":{"input_tokens":3,"output_tokens":4}}"#,
        false,
        2,
        true,
    );
    assert_eq!(r.usage.total_tokens, None);
    assert_eq!(r.usage.cache_read_input_tokens, None);
    assert_eq!(r.completeness(), UsageCompleteness::Partial);
    let r = observe(br#"{"type":"message","usage":{"input_tokens":3,"output_tokens":4,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#, false, 2, true);
    assert_eq!(r.usage.total_tokens, Some(7));
}

#[test]
fn invalid_numbers_are_not_coerced_and_unknown_is_not_zero() {
    for invalid in ["-1", "1.5", "\"7\"", "18446744073709551616", "true"] {
        let body = format!(
            r#"{{"object":"chat.completion","usage":{{"prompt_tokens":{invalid},"completion_tokens":0}}}}"#
        );
        let r = observe(body.as_bytes(), false, 1, true);
        assert!(r.parse_error);
        assert_eq!(r.usage.input_tokens, None);
        assert_eq!(r.usage.output_tokens, Some(0));
        assert_eq!(r.usage.total_tokens, None);
    }
    let r = observe(
        br#"{"object":"chat.completion","usage":null}"#,
        false,
        1,
        true,
    );
    assert_eq!(r.completeness(), UsageCompleteness::Unknown);
}

#[test]
fn errors_and_bad_utf8_are_visible_without_error_body() {
    for body in [
        b"data: not-json\n\n".as_slice(),
        b"data: \xff\n\n",
        b"data: []\n\n",
    ] {
        assert!(observe(body, true, 1, true).parse_error);
    }
    let r = observe(
        br#"{"error":{"message":"private body"},"usage":{"prompt_tokens":10}}"#,
        false,
        1,
        true,
    );
    assert!(r.provider_error);
    assert_eq!(r.completeness(), UsageCompleteness::Unknown);
    assert!(!format!("{r:?}").contains("private body"));
}

#[test]
fn truncated_event_is_never_committed() {
    let event = b"data: {\"usage\":{\"prompt_tokens\":9}}\n";
    let r = observe(event, true, 1, true);
    assert!(r.truncated);
    assert_eq!(r.usage.input_tokens, None);
    let r = observe(b"{}", false, 1, false);
    assert!(r.truncated);
    assert!(observe(b"{", false, 1, false).parse_error);
}

#[test]
fn caps_apply_to_json_lines_and_whole_events() {
    let mut json = b"{}".to_vec();
    json.resize(MAX_JSON_BYTES, b' ');
    assert!(!observe(&json, false, 17, true).limit_exceeded);
    json.push(b' ');
    assert!(observe(&json, false, 17, true).limit_exceeded);
    let line = vec![b'x'; MAX_SSE_BYTES + 1];
    assert!(observe(&line, true, 97, true).limit_exceeded);
    let event = ":comment\n".repeat(MAX_SSE_BYTES / 9 + 1);
    assert!(observe(event.as_bytes(), true, 7, true).limit_exceeded);
    let events = ":comment\n\n".repeat(MAX_JSON_BYTES / 10 + 1);
    assert!(!observe(events.as_bytes(), true, 53, true).limit_exceeded);
}

#[test]
fn unsupported_encoding_and_type_ignore_input() {
    for encoding in [Some("gzip"), Some("br"), Some("identity, gzip"), Some("")] {
        let mut o = UsageObserver::new("application/json", encoding);
        o.feed(&vec![0; MAX_JSON_BYTES + 1]);
        let r = o.finish(true);
        assert_eq!(r.format, ObservationFormat::UnsupportedContentEncoding);
        assert!(!r.limit_exceeded && !r.parse_error);
        assert_eq!(r.completeness(), UsageCompleteness::Unknown);
    }
    let mut o = UsageObserver::new("text/html", None);
    o.feed(b"secret");
    assert_eq!(
        o.finish(true).format,
        ObservationFormat::UnsupportedContentType
    );
}

#[test]
fn overflow_does_not_wrap_and_models_are_filtered() {
    let r = observe(br#"{"type":"message","model":"prompt with spaces","usage":{"input_tokens":18446744073709551615,"output_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}"#, false, 1, true);
    assert!(r.arithmetic_overflow);
    assert_eq!(r.usage.total_tokens, None);
    assert_eq!(r.model, None);
    let body = format!(
        r#"{{"object":"chat.completion","model":"{}","usage":{{"prompt_tokens":1}}}}"#,
        "x".repeat(MAX_MODEL_BYTES + 1)
    );
    assert_eq!(observe(body.as_bytes(), false, 1, true).model, None);
}

#[test]
fn missing_terminal_failed_response_and_nested_usage() {
    let r = observe(
        b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
        true,
        1,
        true,
    );
    assert!(r.truncated);
    assert_eq!(r.usage.input_tokens, Some(1));
    let r = observe(b"event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"secret\"}}}\n\n", true, 1, true);
    assert!(r.provider_error);
    let r = observe(
        br#"{"choices":[{"message":{"content":"secret","usage":{"prompt_tokens":999}}}]}"#,
        false,
        1,
        true,
    );
    assert_eq!(r.completeness(), UsageCompleteness::Unknown);
    assert_eq!(r.model, None);
}

#[test]
fn store_is_bounded_thread_safe_and_preserves_unknowns() {
    let store = std::sync::Arc::new(UsageStore::new());
    let mut threads = Vec::new();
    for thread in 0..4 {
        let store = store.clone();
        threads.push(std::thread::spawn(move || {
            for i in 0..100 {
                store.record(&UsageReport {
                    model: Some(format!("m-{thread}-{i}")),
                    truncated: true,
                    limit_exceeded: true,
                    ..UsageReport::default()
                });
                assert!(store.snapshot().groups.len() <= MAX_GROUPS);
            }
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let s = store.snapshot();
    assert_eq!(s.aggregate.counters.reports, 400);
    assert_eq!(s.aggregate.counters.unknown, 400);
    assert_eq!(s.aggregate.counters.truncated, 400);
    assert_eq!(s.aggregate.counters.parse_limits, 400);
    assert_eq!(s.groups.len(), MAX_GROUPS);
    assert_eq!(s.overflow.counters.reports, 400 - MAX_GROUPS as u64);
    assert_eq!(s.aggregate.observed_tokens.total_tokens, None);
    assert!(s.summary().contains("非计费统计"));
}

#[test]
fn store_counts_each_observation_and_saturates_explicitly() {
    let store = UsageStore::new();
    let r = observe(br#"{"usage":{"prompt_tokens":18446744073709551615,"completion_tokens":0,"total_tokens":18446744073709551615}}"#, false, 1, true);
    store.record(&r);
    store.record(&r);
    let partial = observe(
        br#"{"object":"chat.completion","usage":{"prompt_tokens":0}}"#,
        false,
        1,
        true,
    );
    store.record(&partial);
    let s = store.snapshot();
    assert_eq!(s.aggregate.counters.complete, 2);
    assert_eq!(s.aggregate.counters.partial, 1);
    assert!(s.aggregate.sum_overflow);
    assert_eq!(s.aggregate.observed_tokens.total_tokens, Some(u64::MAX));
    assert_eq!(s.groups.len(), 1);
}
