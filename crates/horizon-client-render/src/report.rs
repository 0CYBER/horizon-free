//! authlib's abuse-reporting service without its screens: `UserApiService`'s
//! `reportAbuse`, `canSendReports` and `getAbuseReportLimits`, and the request
//! the client's `AbuseReportSender.send` builds
//! (`net.minecraft.client.multiplayer.chat.report.AbuseReportSender`).
//!
//! The wire types carry authlib's gson shapes: record component names in
//! camelCase, `null` fields omitted (gson's default), UUIDs undashed
//! (`UUIDTypeAdapter`), instants ISO-8601 (`InstantTypeAdapter`) and
//! `ByteBuffer`s base64 (`ByteBufferTypeAdapter`). The screens that collect
//! the evidence and call `send_report` are a later part, as in vanilla where
//! they live outside authlib.
use crate::auth;
use serde::Serialize;
use std::time::Duration;

/// authlib's `AbuseReportLimits`: how big a report may be.
/// `getAbuseReportLimits` returns `DEFAULTS` whatever the account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbuseReportLimits {
    pub max_opinion_comments_length: i32,
    pub max_reported_message_count: i32,
    pub max_evidence_message_count: i32,
    pub leading_context_message_count: i32,
    pub trailing_context_message_count: i32,
}
impl AbuseReportLimits {
    /// `AbuseReportLimits.DEFAULTS`: `new AbuseReportLimits(1000, 4, 40, 9, 0)`.
    pub const DEFAULTS: Self = Self {
        max_opinion_comments_length: 1000,
        max_reported_message_count: 4,
        max_evidence_message_count: 40,
        leading_context_message_count: 9,
        trailing_context_message_count: 0,
    };
}

/// authlib's `AbuseReportRequest.ClientInfo` (`ReportEnvironment.clientInfo`):
/// the client's version string and the locale's language tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub client_version: String,
    pub locale: String,
}

/// authlib's `AbuseReportRequest.ThirdPartyServerInfo`:
/// `ReportEnvironment.thirdParty`'s server address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ThirdPartyServerInfo {
    pub address: String,
}

/// authlib's `AbuseReportRequest.RealmInfo`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmInfo {
    pub realm_id: String,
    pub slot_id: i32,
}

/// authlib's `ReportedEntity`: the reported account, undashed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportedEntity {
    pub profile_id: String,
}

/// authlib's `ReportChatMessage`: one signed line of the evidence. The
/// signatures are base64, the timestamp ISO-8601, as the gson adapters write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportChatMessage {
    pub index: i32,
    pub profile_id: String,
    pub session_id: String,
    pub timestamp: String,
    pub salt: i64,
    pub last_seen: Vec<String>,
    pub message: String,
    pub signature: String,
    pub message_reported: bool,
}

/// authlib's `ReportEvidence`: the reported line and its context.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReportEvidence {
    pub messages: Vec<ReportChatMessage>,
}

/// authlib's `AbuseReport`: one record for the three report kinds, whose
/// factory methods null the fields the kind does not carry (gson omits them).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbuseReport {
    pub opinion_comments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<ReportEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin_url: Option<String>,
    pub reported_entity: ReportedEntity,
    /// `InstantTypeAdapter`'s ISO-8601 form of `createdTime`.
    pub created_time: String,
}
impl AbuseReport {
    /// `AbuseReport.name`: a username report — no reason, evidence or skin URL.
    pub fn name(
        opinion_comments: String,
        reported_entity: ReportedEntity,
        created_time: String,
    ) -> Self {
        Self {
            opinion_comments,
            reason: None,
            evidence: None,
            skin_url: None,
            reported_entity,
            created_time,
        }
    }

    /// `AbuseReport.skin`: a skin report — no evidence.
    pub fn skin(
        opinion_comments: String,
        reason: String,
        skin_url: String,
        reported_entity: ReportedEntity,
        created_time: String,
    ) -> Self {
        Self {
            opinion_comments,
            reason: Some(reason),
            evidence: None,
            skin_url: Some(skin_url),
            reported_entity,
            created_time,
        }
    }

    /// `AbuseReport.chat`: a chat report — no skin URL.
    pub fn chat(
        opinion_comments: String,
        reason: String,
        evidence: ReportEvidence,
        reported_entity: ReportedEntity,
        created_time: String,
    ) -> Self {
        Self {
            opinion_comments,
            reason: Some(reason),
            evidence: Some(evidence),
            skin_url: None,
            reported_entity,
            created_time,
        }
    }
}

/// authlib's `AbuseReportRequest`: `AbuseReportSender.send` builds it with
/// version 1, a fresh report id, the report, where the client runs, and the
/// report type's backend name (`CHAT`, `SKIN` or `USERNAME`). The null
/// environment fields are the local report's, omitted like gson's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbuseReportRequest {
    pub version: i32,
    /// The report's own id, undashed.
    pub id: String,
    pub report: AbuseReport,
    pub client_info: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub third_party_server_info: Option<ThirdPartyServerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realm_info: Option<RealmInfo>,
    pub report_type: String,
}
impl AbuseReportRequest {
    /// `AbuseReportSender.send`: version 1, with `reportType.backendName()`.
    pub fn new(
        id: String,
        report: AbuseReport,
        client_info: ClientInfo,
        third_party_server_info: Option<ThirdPartyServerInfo>,
        realm_info: Option<RealmInfo>,
        report_type: &str,
    ) -> Self {
        Self {
            version: 1,
            id,
            report,
            client_info,
            third_party_server_info,
            realm_info,
            report_type: report_type.to_owned(),
        }
    }
}

/// `UserApiService.getAbuseReportLimits`: always the service's `DEFAULTS`.
pub fn abuse_report_limits() -> AbuseReportLimits {
    AbuseReportLimits::DEFAULTS
}

/// `AbuseReportSender.Services.isEnabled` (`UserApiService.canSendReports`):
/// the online service's answer is always yes; without a signed-in account
/// vanilla is on the offline service, whose answer is no.
pub fn can_send_reports() -> bool {
    auth::current().is_some()
}

/// `UserApiService.reportAbuse`: the request goes to the player service's
/// `sendReport` endpoint. The error is the service's answer, as the
/// `MinecraftClientHttpException` the sender catches.
pub fn send_report(request: &AbuseReportRequest) -> Result<(), String> {
    let session = auth::current().ok_or("no signed-in account")?;
    let doc = auth::discovery().map_err(|_| "the services discovery did not answer".to_owned())?;
    let url = auth::endpoint(&doc, "player", "sendReport")
        .ok_or("the discovery document has no sendReport endpoint".to_owned())?;
    let body = serde_json::to_string(request).map_err(|e| e.to_string())?;
    submit_report(&url, session.access_token(), &body)
}

/// The POST itself, split from `send_report`'s assembly: no globals, so the
/// redirect rules are testable against a loopback server. Redirects are off —
/// ureq re-POSTs nowhere and, on a 307/308, answers the redirect itself;
/// 301/302/303 would otherwise turn the POST into a GET whose 2xx would look
/// like acceptance while the report body never reached the service. Only a
/// 2xx from the endpoint counts. The agent carries `auth::agent`'s timeout
/// and user agent with its own redirect count.
fn submit_report(url: &str, bearer: &str, body: &str) -> Result<(), String> {
    static AGENT: std::sync::LazyLock<ureq::Agent> = std::sync::LazyLock::new(|| {
        ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(Duration::from_secs(15))
            .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
            .build()
    });
    match AGENT
        .post(url)
        .set("Authorization", &format!("Bearer {bearer}"))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(body)
    {
        Ok(response) => {
            // ureq answers below 400 as Ok — including the redirects this
            // request refused. Vanilla's MinecraftClient accepts only 2xx.
            if (200..300).contains(&response.status()) {
                Ok(())
            } else {
                Err(format!("the report service answered {}", response.status()))
            }
        }
        Err(ureq::Error::Status(status, response)) => {
            let message = response
                .into_string()
                .ok()
                .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
                .and_then(|v| {
                    v.get("errorMessage")
                        .and_then(|m| m.as_str())
                        .map(str::to_owned)
                });
            Err(message.unwrap_or_else(|| format!("the report service answered {status}")))
        }
        Err(ureq::Error::Transport(e)) => Err(format!("the report service: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity() -> ReportedEntity {
        ReportedEntity {
            profile_id: "069a79f444e94726a5befca90e38aaf5".to_owned(),
        }
    }

    /// A chat report as `AbuseReportSender.send` sends it: camelCase names,
    /// undashed ids, the local report's null environment omitted.
    #[test]
    fn a_chat_report_serializes_as_the_service_reads_it() {
        let evidence = ReportEvidence {
            messages: vec![ReportChatMessage {
                index: 7,
                profile_id: "e9970cf5211e399895ba4a5b7ea7fe06".to_owned(),
                session_id: "00000000000000000000000000000000".to_owned(),
                timestamp: "2026-11-19T12:00:00.123456Z".to_owned(),
                salt: 1234567890,
                last_seen: vec!["AAEC".to_owned()],
                message: "hello".to_owned(),
                signature: "AA==".to_owned(),
                message_reported: true,
            }],
        };
        let request = AbuseReportRequest::new(
            "6b0f0a0e000040008000000000000000".to_owned(),
            AbuseReport::chat(
                "opinion".to_owned(),
                "hate_speech".to_owned(),
                evidence,
                entity(),
                "2026-11-19T12:01:00Z".to_owned(),
            ),
            ClientInfo {
                client_version: "26.3 (modded)".to_owned(),
                locale: "en-US".to_owned(),
            },
            Some(ThirdPartyServerInfo {
                address: "play.example.net".to_owned(),
            }),
            None,
            "CHAT",
        );
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            json,
            r#"{"version":1,"id":"6b0f0a0e000040008000000000000000","report":{"opinionComments":"opinion","reason":"hate_speech","evidence":{"messages":[{"index":7,"profileId":"e9970cf5211e399895ba4a5b7ea7fe06","sessionId":"00000000000000000000000000000000","timestamp":"2026-11-19T12:00:00.123456Z","salt":1234567890,"lastSeen":["AAEC"],"message":"hello","signature":"AA==","messageReported":true}]},"reportedEntity":{"profileId":"069a79f444e94726a5befca90e38aaf5"},"createdTime":"2026-11-19T12:01:00Z"},"clientInfo":{"clientVersion":"26.3 (modded)","locale":"en-US"},"thirdPartyServerInfo":{"address":"play.example.net"},"reportType":"CHAT"}"#
        );
    }

    /// The report kinds null what they do not carry, and gson omits those
    /// fields: a username report has no reason, evidence or skin URL, and a
    /// skin report no evidence.
    #[test]
    fn the_report_kinds_leave_their_unused_fields_out() {
        let json = serde_json::to_string(&AbuseReport::name(
            "opinion".to_owned(),
            entity(),
            "2026-11-19T12:01:00Z".to_owned(),
        ))
        .unwrap();
        assert_eq!(
            json,
            r#"{"opinionComments":"opinion","reportedEntity":{"profileId":"069a79f444e94726a5befca90e38aaf5"},"createdTime":"2026-11-19T12:01:00Z"}"#
        );
        let skin = AbuseReport::skin(
            "opinion".to_owned(),
            "inappropriate_skin".to_owned(),
            "https://textures.minecraft.net/texture/x".to_owned(),
            entity(),
            "2026-11-19T12:01:00Z".to_owned(),
        );
        let json = serde_json::to_string(&skin).unwrap();
        assert!(!json.contains("evidence"));
        assert!(json.contains(r#""skinUrl":"https://textures.minecraft.net/texture/x""#));
    }

    /// `getAbuseReportLimits`' constants are the service's `DEFAULTS`.
    #[test]
    fn limits_are_the_service_defaults() {
        let limits = abuse_report_limits();
        assert_eq!(limits.max_opinion_comments_length, 1000);
        assert_eq!(limits.max_reported_message_count, 4);
        assert_eq!(limits.max_evidence_message_count, 40);
        assert_eq!(limits.leading_context_message_count, 9);
        assert_eq!(limits.trailing_context_message_count, 0);
    }

    /// One loopback server answering the four routes a redirect test needs.
    fn loopback() -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let routes = [
                (
                    "POST /moved",
                    "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n",
                ),
                (
                    "POST /final",
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
                ),
                (
                    "POST /accepted",
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
                ),
                (
                    "POST /refused",
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: 47\r\n\r\n{\"error\":\"X\",\"errorMessage\":\"the reason field\"}",
                ),
            ];
            for _ in 0..3 {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let route = request
                    .split("\r\n")
                    .next()
                    .and_then(|line| routes.iter().find(|(name, _)| line.starts_with(name)));
                let answer = route
                    .map(|(_, answer)| *answer)
                    .unwrap_or("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                let _ = stream.write_all(answer.as_bytes());
                // Half-close and wait out the client's read: dropping now
                // resets the connection mid-answer.
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let _ = stream.read(&mut buffer);
            }
        });
        format!("http://{address}")
    }

    /// Redirects are off: the 302 is the answer (an error, the body never
    /// went to /final), the endpoint's 2xx is acceptance, and the service's
    /// `errorMessage` names the error.
    #[test]
    fn submission_accepts_only_the_endpoints_answer() {
        let base = loopback();
        assert_eq!(
            submit_report(&format!("{base}/moved"), "bearer", "{}"),
            Err("the report service answered 302".to_owned()),
        );
        assert_eq!(
            submit_report(&format!("{base}/accepted"), "bearer", "{}"),
            Ok(())
        );
        assert_eq!(
            submit_report(&format!("{base}/refused"), "bearer", "{}"),
            Err("the reason field".to_owned()),
        );
    }
}
