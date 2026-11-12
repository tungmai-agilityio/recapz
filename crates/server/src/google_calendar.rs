//! Google Calendar events.list responses turned into calendar rows. Pure parsing and request URL
//! building; the HTTP call and the sign-in scope that allows it are wired separately.
use recapz_core::calendar::NewEvent;
use recapz_notetaker::links;
use serde_json::Value;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const MAX_ID: usize = 1024;

pub struct Page {
    pub events: Vec<NewEvent>,
    pub next_page_token: Option<String>,
}

fn ms(v: &Value) -> Option<i64> {
    let t = OffsetDateTime::parse(v.get("dateTime")?.as_str()?, &Rfc3339).ok()?;
    Some((t.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// A video entry point, then the Meet link Google attaches, then a link in the location or notes.
fn join_link(e: &Value) -> Option<links::Link> {
    let entry_points = e
        .pointer("/conferenceData/entryPoints")
        .and_then(Value::as_array);
    entry_points
        .into_iter()
        .flatten()
        .filter(|p| p.get("entryPointType").and_then(Value::as_str) == Some("video"))
        .filter_map(|p| p.get("uri").and_then(Value::as_str))
        .find_map(links::parse)
        .or_else(|| {
            e.get("hangoutLink")
                .and_then(Value::as_str)
                .and_then(links::parse)
        })
        .or_else(|| {
            e.get("location")
                .and_then(Value::as_str)
                .and_then(links::find_in_text)
        })
        .or_else(|| {
            e.get("description")
                .and_then(Value::as_str)
                .and_then(links::find_in_text)
        })
}

/// Cancelled events, all-day events and events without a start and end time are skipped.
pub fn parse_page(body: &str) -> Result<Page, serde_json::Error> {
    let v: Value = serde_json::from_str(body)?;
    let mut events = Vec::new();
    for e in v
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if e.get("status").and_then(Value::as_str) == Some("cancelled") {
            continue;
        }
        let (Some(id), Some(start), Some(end)) = (
            e.get("id").and_then(Value::as_str),
            e.get("start").and_then(ms),
            e.get("end").and_then(ms),
        ) else {
            continue;
        };
        // Out-of-office, focus time and working location entries are not meetings; an absent type
        // means a normal event. Odd ids and backwards times are dropped too.
        let kind = e
            .get("eventType")
            .and_then(Value::as_str)
            .unwrap_or("default");
        if kind != "default" || id.len() > MAX_ID || end < start {
            continue;
        }
        let link = join_link(e);
        events.push(NewEvent {
            external_id: id.to_string(),
            title: e
                .get("summary")
                .and_then(Value::as_str)
                .map(|s| s.trim().chars().take(200).collect::<String>())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "(No title)".to_string()),
            starts_at: start,
            ends_at: end,
            platform: link.as_ref().map(|l| l.platform().to_string()),
            join_url: link.map(|l| l.url().to_string()),
        });
    }
    Ok(Page {
        events,
        next_page_token: v
            .get("nextPageToken")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Query pairs for the primary calendar: recurring events expanded, ordered by start time.
pub fn list_query(
    from_ms: i64,
    to_ms: i64,
    page_token: Option<&str>,
) -> Vec<(&'static str, String)> {
    let rfc = |ms: i64| {
        OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
            .ok()
            .and_then(|t| t.format(&Rfc3339).ok())
            .unwrap_or_default()
    };
    let mut q = vec![
        ("timeMin", rfc(from_ms)),
        ("timeMax", rfc(to_ms)),
        ("singleEvents", "true".to_string()),
        ("orderBy", "startTime".to_string()),
        ("maxResults", "250".to_string()),
    ];
    if let Some(t) = page_token {
        q.push(("pageToken", t.to_string()));
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"{
      "nextPageToken": "tok2",
      "items": [
        {"id":"a","summary":"  Standup ","start":{"dateTime":"2026-11-12T09:00:00+07:00"},"end":{"dateTime":"2026-11-12T09:30:00+07:00"},
         "hangoutLink":"https://meet.google.com/abc-defg-hij"},
        {"id":"b","start":{"dateTime":"2026-11-12T10:00:00Z"},"end":{"dateTime":"2026-11-12T11:00:00Z"},
         "location":"Room 3, or https://us02web.zoom.us/j/84012345678?pwd=xyz#success"},
        {"id":"c","summary":"Offsite","start":{"date":"2026-11-13"},"end":{"date":"2026-11-14"}},
        {"id":"d","status":"cancelled","summary":"Gone","start":{"dateTime":"2026-11-12T12:00:00Z"},"end":{"dateTime":"2026-11-12T13:00:00Z"}},
        {"id":"e","summary":"Lunch","start":{"dateTime":"2026-11-12T12:00:00Z"},"end":{"dateTime":"2026-11-12T13:00:00Z"},
         "description":"see https://evil.example/meet.google.com/abc-defg-hij"},
        {"id":"f","eventType":"outOfOffice","start":{"dateTime":"2026-11-12T12:00:00Z"},"end":{"dateTime":"2026-11-12T13:00:00Z"}},
        {"id":"g","eventType":"default","summary":"Backwards","start":{"dateTime":"2026-11-12T13:00:00Z"},"end":{"dateTime":"2026-11-12T12:00:00Z"}},
        {"id":"h","summary":"Bad time","start":{"dateTime":"tomorrow"},"end":{"dateTime":"2026-11-12T12:00:00Z"}},
        {"summary":"no id","start":{"dateTime":"2026-11-12T12:00:00Z"},"end":{"dateTime":"2026-11-12T13:00:00Z"}}
      ]}"#;

    #[test]
    fn maps_events_and_skips_all_day_cancelled_and_incomplete_ones() {
        let p = parse_page(PAGE).unwrap();
        assert_eq!(p.next_page_token.as_deref(), Some("tok2"));
        type Row<'a> = (&'a str, &'a str, i64, i64, Option<&'a str>, Option<&'a str>);
        let got: Vec<Row> = p
            .events
            .iter()
            .map(|e| {
                (
                    e.external_id.as_str(),
                    e.title.as_str(),
                    e.starts_at,
                    e.ends_at,
                    e.platform.as_deref(),
                    e.join_url.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "a",
                    "Standup",
                    1_794_448_800_000,
                    1_794_450_600_000,
                    Some("meet"),
                    Some("https://meet.google.com/abc-defg-hij")
                ),
                (
                    "b",
                    "(No title)",
                    1_794_477_600_000,
                    1_794_481_200_000,
                    Some("zoom"),
                    Some("https://us02web.zoom.us/j/84012345678?pwd=xyz")
                ),
                (
                    "e",
                    "Lunch",
                    1_794_484_800_000,
                    1_794_488_400_000,
                    None,
                    None
                ),
            ]
        );
    }

    #[test]
    fn prefers_the_conference_entry_point_and_ignores_non_video_ones() {
        let body = r#"{"items":[{"id":"x","start":{"dateTime":"2026-11-12T09:00:00Z"},"end":{"dateTime":"2026-11-12T10:00:00Z"},
          "hangoutLink":"https://meet.google.com/aaa-bbbb-ccc",
          "conferenceData":{"entryPoints":[
            {"entryPointType":"phone","uri":"tel:+1555"},
            {"entryPointType":"video","uri":"https://meet.google.com/ddd-eeee-fff"}]}}]}"#;
        let p = parse_page(body).unwrap();
        assert_eq!(
            p.events[0].join_url.as_deref(),
            Some("https://meet.google.com/ddd-eeee-fff")
        );
        assert_eq!(p.next_page_token, None);
    }

    #[test]
    fn bad_json_is_an_error_and_an_empty_calendar_is_fine() {
        assert!(parse_page("nope").is_err());
        assert!(parse_page("{}").unwrap().events.is_empty());
    }

    #[test]
    fn query_has_the_window_and_the_page_token() {
        let q = list_query(1_794_448_800_000, 1_794_535_200_000, Some("tok"));
        let get = |k: &str| q.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("timeMin"), Some("2026-11-12T02:00:00Z"));
        assert_eq!(get("timeMax"), Some("2026-11-13T02:00:00Z"));
        assert_eq!(
            (get("singleEvents"), get("orderBy"), get("pageToken")),
            (Some("true"), Some("startTime"), Some("tok"))
        );
        assert_eq!(
            list_query(0, 1, None)
                .iter()
                .filter(|(n, _)| *n == "pageToken")
                .count(),
            0
        );
    }

    #[test]
    fn link_priority_falls_back_through_hangout_location_and_description() {
        let ev = |extra: &str| {
            let body = format!(
                r#"{{"items":[{{"id":"x","start":{{"dateTime":"2026-11-12T09:00:00Z"}},"end":{{"dateTime":"2026-11-12T10:00:00Z"}},{extra}}}]}}"#
            );
            parse_page(&body).unwrap().events.remove(0).join_url
        };
        // An entry point that is not a Meet or Zoom link is ignored; hangoutLink is next.
        assert_eq!(
            ev(r#""conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://other.example/x"}]},"hangoutLink":"https://meet.google.com/aaa-bbbb-ccc""#).as_deref(),
            Some("https://meet.google.com/aaa-bbbb-ccc")
        );
        // Location beats description; description is the last resort.
        assert_eq!(
            ev(r#""location":"https://zoom.us/j/123456789","description":"https://meet.google.com/aaa-bbbb-ccc""#).as_deref(),
            Some("https://zoom.us/j/123456789")
        );
        assert_eq!(
            ev(r#""description":"join https://meet.google.com/aaa-bbbb-ccc please""#).as_deref(),
            Some("https://meet.google.com/aaa-bbbb-ccc")
        );
    }

    #[test]
    fn titles_and_ids_are_bounded() {
        let long_id = "i".repeat(MAX_ID + 1);
        let body = format!(
            r#"{{"items":[{{"id":"ok","summary":"{}","start":{{"dateTime":"2026-11-12T09:00:00Z"}},"end":{{"dateTime":"2026-11-12T10:00:00Z"}}}},
                 {{"id":"{long_id}","start":{{"dateTime":"2026-11-12T09:00:00Z"}},"end":{{"dateTime":"2026-11-12T10:00:00Z"}}}},
                 {{"id":"blank","summary":"   ","start":{{"dateTime":"2026-11-12T09:00:00Z"}},"end":{{"dateTime":"2026-11-12T10:00:00Z"}}}}]}}"#,
            "t".repeat(300)
        );
        let p = parse_page(&body).unwrap();
        assert_eq!(p.events.len(), 2, "the long id is dropped");
        assert_eq!(
            (p.events[0].title.len(), p.events[1].title.as_str()),
            (200, "(No title)")
        );
    }
}
