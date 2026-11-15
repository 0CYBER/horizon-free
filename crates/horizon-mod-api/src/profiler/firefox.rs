//! Raw Gecko profile format 37, from the public format declarations and marker guide:
//! <https://firefox-source-docs.mozilla.org/tools/profiler/markers-guide.html>
//! <https://github.com/firefox-devtools/profiler/blob/main/docs-developer/CHANGELOG-formats.md>
//! Aggregate snapshots are instant markers at synthetic time zero, not invocation intervals.
//! Empty samples deliberately provide no invented call tree or execution-time shares.
use super::Report;
use serde::Serialize;
use std::io::{self, Write};

pub(super) const MAX_BYTES: usize = 8 * 1024 * 1024;
const HEADER: &str = r#"{
"meta":{"version":37,"startTime":0,"shutdownTime":null,"profilingStartTime":0,
"profilingEndTime":0,"interval":1,"stackwalk":0,"debug":0,"gcpoison":0,"asyncstack":0,
"processType":0,"product":"Horizon aggregate mod profiler (inclusive; no samples)",
"categories":[{"name":"Other","color":"grey","subcategories":["Other"]}],
"markerSchema":[{"name":"HorizonModAggregate","display":["marker-chart","marker-table"],
"tableLabel":"{marker.data.mod} / {marker.data.hook} / {marker.data.part}","data":[
{"key":"mod","label":"Mod","format":"string"},
{"key":"hook","label":"Hook","format":"string"},
{"key":"part","label":"Part","format":"string"},
{"key":"count","label":"Invocations","format":"string"},
{"key":"totalNs","label":"Inclusive total (ns)","format":"string"},
{"key":"minNs","label":"Minimum (ns)","format":"string"},
{"key":"maxNs","label":"Maximum (ns)","format":"string"},
{"key":"histogram","label":"Log2 buckets (ns; bucket 0 is 0/1)","format":"string"},
{"key":"droppedCalls","label":"Dropped calls in this session","format":"string"}]}]},
"libs":[],"pages":[],"processes":[],"pausedRanges":[],
"sources":{"schema":{"id":0,"filename":1,"startLine":2,"startColumn":3,"sourceMapURL":4},"data":[]},
"threads":[{"name":"Mod aggregates","processType":"default","processName":"Horizon",
"pid":1,"tid":1,"registerTime":0,"unregisterTime":null,
"stringTable":["Aggregate mod call timings (inclusive)"],
"samples":{"schema":{"stack":0,"time":1,"eventDelay":2},"data":[]},
"frameTable":{"schema":{"location":0,"relevantForJS":1,"innerWindowID":2,"implementation":3,
"line":4,"column":5,"category":6,"subcategory":7},"data":[]},
"stackTable":{"schema":{"prefix":0,"frame":1},"data":[]},
"markers":{"schema":{"name":0,"startTime":1,"endTime":2,"phase":3,"category":4,"data":5},"data":[
"#;
const FOOTER: &str = "]}}]}";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Payload<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "mod")]
    mod_id: &'a str,
    hook: &'a str,
    part: &'a str,
    // Decimal strings preserve exact u64/u128 values beyond JavaScript's 2^53 range.
    count: String,
    total_ns: String,
    min_ns: String,
    max_ns: String,
    histogram: String,
    dropped_calls: String,
}
struct Bounded {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other(format!(
                "Firefox profile exceeds {} bytes",
                self.limit
            )));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(report: &Report) -> Result<Vec<u8>, String> {
    encode_limited(report, MAX_BYTES).map_err(|e| e.to_string())
}
fn encode_limited(report: &Report, limit: usize) -> io::Result<Vec<u8>> {
    let mut out = Bounded {
        bytes: Vec::new(),
        limit,
    };
    out.write_all(HEADER.as_bytes())?;
    for (index, ((id, hook, part), a)) in report.calls.iter().enumerate() {
        if index != 0 {
            out.write_all(b",")?;
        }
        serde_json::to_writer(
            &mut out,
            &(
                0,
                0,
                None::<u8>,
                0,
                0,
                Payload {
                    kind: "HorizonModAggregate",
                    mod_id: id,
                    hook,
                    part: part.unwrap_or("-"),
                    count: a.count.to_string(),
                    total_ns: a.total.to_string(),
                    min_ns: a.min.to_string(),
                    max_ns: a.max.to_string(),
                    histogram: serde_json::to_string(&a.histogram[..])?,
                    dropped_calls: report.dropped.to_string(),
                },
            ),
        )?;
    }
    out.write_all(FOOTER.as_bytes())?;
    Ok(out.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::__private::profiler::Record;
    use serde_json::{Value, json};
    fn fixed_report() -> Report {
        let mut report = Report {
            dropped: 3,
            ..Report::default()
        };
        for nanoseconds in [2, 8, 8] {
            report.record(Record {
                mod_id: "a\"\\\n\t\u{1}é",
                hook: "tick",
                part: Some("Energy"),
                nanoseconds,
            });
        }
        for _ in 0..2 {
            report.record(Record {
                mod_id: "z",
                hook: "part",
                part: None,
                nanoseconds: u64::MAX,
            });
        }
        report
    }
    #[test]
    fn profiler_firefox_fixed_report() {
        let bytes = encode(&fixed_report()).unwrap();
        let profile = serde_json::from_slice::<Value>(&bytes);
        assert!(
            profile.is_ok(),
            "encoded names must remain valid JSON: {profile:?}"
        );
        let profile = profile.unwrap();
        assert_eq!(profile["meta"]["version"], 37);
        assert_eq!(profile["meta"]["startTime"], 0);
        assert_eq!(
            profile["meta"]["markerSchema"][0]["name"],
            "HorizonModAggregate"
        );
        let schema = profile["meta"]["markerSchema"][0]["data"]
            .as_array()
            .unwrap();
        assert_eq!(
            schema
                .iter()
                .map(|f| f["key"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "mod",
                "hook",
                "part",
                "count",
                "totalNs",
                "minNs",
                "maxNs",
                "histogram",
                "droppedCalls"
            ]
        );
        assert!(schema.iter().all(|f| f["format"] == "string"));
        assert_eq!(profile["threads"].as_array().unwrap().len(), 1);
        let thread = &profile["threads"][0];
        assert_eq!(
            thread["markers"]["schema"],
            json!({"name":0,"startTime":1,"endTime":2,"phase":3,"category":4,"data":5})
        );
        assert_eq!(thread["samples"]["data"], json!([]));
        assert_eq!(thread["frameTable"]["data"], json!([]));
        assert_eq!(thread["stackTable"]["data"], json!([]));
        let rows = thread["markers"]["data"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            &rows[0].as_array().unwrap()[..5],
            &[json!(0), json!(0), Value::Null, json!(0), json!(0)]
        );
        let mut buckets = vec![0; 64];
        buckets[1] = 1;
        buckets[3] = 2;
        assert_eq!(
            rows[0][5],
            json!({"type":"HorizonModAggregate", "mod":"a\"\\\n\t\u{1}é", "hook":"tick", "part":"Energy", "count":"3", "totalNs":"18", "minNs":"2", "maxNs":"8", "histogram":serde_json::to_string(&buckets).unwrap(), "droppedCalls":"3"})
        );
        assert_eq!(rows[1][5]["part"], "-");
        assert_eq!(rows[1][5]["totalNs"], "36893488147419103230");
        assert_eq!(rows[1][5]["maxNs"], "18446744073709551615");
        assert_eq!(
            encode(&fixed_report()).unwrap(),
            encode(&fixed_report()).unwrap()
        );
    }
    #[test]
    fn profiler_firefox_limit_and_empty() {
        let report = fixed_report();
        let bytes = encode(&report).unwrap();
        assert!(encode_limited(&report, bytes.len() - 1).is_err());
        assert_eq!(encode_limited(&report, bytes.len()).unwrap(), bytes);
        let mut oversized = Report::default();
        oversized.record(Record {
            mod_id: Box::leak("m".repeat(8_388_608).into_boxed_str()),
            hook: "h",
            part: None,
            nanoseconds: 1,
        });
        assert!(encode(&oversized).unwrap_err().contains("8388608 bytes"));
        let profile: Value = serde_json::from_slice(&encode(&Report::default()).unwrap()).unwrap();
        assert_eq!(profile["threads"][0]["markers"]["data"], json!([]));
    }
}
