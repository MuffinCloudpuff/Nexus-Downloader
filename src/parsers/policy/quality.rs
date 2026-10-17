use serde_json::Value;

pub const DEFAULT_BILIBILI_MAX_VIDEO_QN: u64 = 80;

pub fn bilibili_stream_video_qn(stream: &Value) -> Option<u64> {
    stream["id"].as_u64()
}

pub fn bilibili_stream_bandwidth(stream: &Value) -> u64 {
    stream["bandwidth"].as_u64().unwrap_or(0)
}

pub fn select_bilibili_video_stream(streams: &[Value], max_qn: u64) -> Option<&Value> {
    streams
        .iter()
        .filter(|stream| {
            bilibili_stream_video_qn(stream)
                .map(|qn| qn <= max_qn)
                .unwrap_or(true)
        })
        .max_by_key(|stream| {
            (
                bilibili_stream_video_qn(stream).unwrap_or(0),
                bilibili_stream_bandwidth(stream),
            )
        })
        .or_else(|| {
            streams
                .iter()
                .min_by_key(|stream| bilibili_stream_bandwidth(stream))
        })
}

pub fn select_highest_bandwidth_index<T>(
    variants: &[T],
    bandwidth: impl Fn(&T) -> u64,
) -> Option<usize> {
    variants
        .iter()
        .enumerate()
        .max_by_key(|(_, variant)| bandwidth(variant))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bilibili_quality_prefers_highest_qn_under_cap_then_bandwidth() {
        let streams = vec![
            json!({"id": 120, "bandwidth": 10_000}),
            json!({"id": 80, "bandwidth": 2_000}),
            json!({"id": 80, "bandwidth": 3_000}),
            json!({"id": 64, "bandwidth": 5_000}),
        ];

        let selected = select_bilibili_video_stream(&streams, DEFAULT_BILIBILI_MAX_VIDEO_QN)
            .expect("stream selected");
        assert_eq!(bilibili_stream_video_qn(selected), Some(80));
        assert_eq!(bilibili_stream_bandwidth(selected), 3_000);
    }

    #[test]
    fn bilibili_quality_falls_back_to_lowest_bandwidth_when_all_above_cap() {
        let streams = vec![
            json!({"id": 120, "bandwidth": 10_000}),
            json!({"id": 116, "bandwidth": 8_000}),
        ];

        let selected = select_bilibili_video_stream(&streams, DEFAULT_BILIBILI_MAX_VIDEO_QN)
            .expect("fallback stream selected");
        assert_eq!(bilibili_stream_video_qn(selected), Some(116));
    }

    #[test]
    fn highest_bandwidth_variant_returns_index() {
        let variants = [100_u64, 300, 200];
        assert_eq!(
            select_highest_bandwidth_index(&variants, |value| *value),
            Some(1)
        );
    }
}
