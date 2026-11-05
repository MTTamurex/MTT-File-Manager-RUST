use mtt_search_protocol::{SearchRequest, SearchResponse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Pass,
    Accepted,
    Inconclusive,
}

impl Outcome {
    fn combine(self, next: Self) -> Self {
        match (self, next) {
            (Self::Accepted, _) | (_, Self::Accepted) => Self::Accepted,
            (Self::Inconclusive, _) | (_, Self::Inconclusive) => Self::Inconclusive,
            _ => Self::Pass,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Query,
    FolderSize,
    CheckPathsModified,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Query => "Query",
            Self::FolderSize => "FolderSize",
            Self::CheckPathsModified => "CheckPathsModified",
        }
    }
}

pub(super) struct Check {
    pub kind: Kind,
    pub request: SearchRequest,
}

fn classify(kind: Kind, response: &SearchResponse) -> Outcome {
    if matches!(response, SearchResponse::Error(message) if message == "Authorization failed") {
        return Outcome::Pass;
    }
    match (kind, response) {
        (Kind::Query, SearchResponse::Results { .. })
        | (Kind::FolderSize, SearchResponse::FolderSize { .. })
        | (Kind::CheckPathsModified, SearchResponse::PathsModified { .. }) => Outcome::Accepted,
        _ => Outcome::Inconclusive,
    }
}

pub(super) fn run(
    checks: [Check; 3],
    mut request: impl FnMut(&SearchRequest) -> Result<SearchResponse, &'static str>,
) -> Outcome {
    match request(&SearchRequest::Ping) {
        Ok(SearchResponse::Pong) => println!("PASS Ping: registered service responded."),
        Ok(_) => {
            println!("INCONCLUSIVE Ping: expected public Pong response.");
            return Outcome::Inconclusive;
        }
        Err(reason) => {
            println!("INCONCLUSIVE Ping: {reason}");
            return Outcome::Inconclusive;
        }
    }
    let mut overall = Outcome::Pass;
    for check in checks {
        let label = check.kind.label();
        let outcome = match request(&check.request) {
            Ok(response) => classify(check.kind, &response),
            Err(reason) => {
                println!("INCONCLUSIVE {label}: {reason}");
                overall = overall.combine(Outcome::Inconclusive);
                continue;
            }
        };
        match outcome {
            Outcome::Pass => println!("PASS {label}: diagnostic explicitly denied authorization."),
            Outcome::Accepted => println!(
                "FAIL {label}: endpoint accepted diagnostic, even if result is empty; contents not displayed."
            ),
            Outcome::Inconclusive => println!(
                "INCONCLUSIVE {label}: response was not an explicit authorization denial."
            ),
        }
        overall = overall.combine(outcome);
    }
    overall
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    fn accepted_responses() -> [(Kind, SearchResponse); 3] {
        [
            (
                Kind::Query,
                SearchResponse::Results {
                    items: vec![],
                    has_more: false,
                    total_matches: Some(0),
                },
            ),
            (
                Kind::FolderSize,
                SearchResponse::FolderSize {
                    path: r"C:\Example\empty-folder".into(),
                    total_size: 0,
                    file_count: 0,
                    folder_count: 0,
                },
            ),
            (
                Kind::CheckPathsModified,
                SearchResponse::PathsModified { modified: vec![] },
            ),
        ]
    }

    #[test]
    fn only_explicit_authorization_denial_passes_each_check() {
        for kind in [Kind::Query, Kind::FolderSize, Kind::CheckPathsModified] {
            assert_eq!(
                classify(kind, &SearchResponse::Error("Authorization failed".into())),
                Outcome::Pass
            );
            for response in [
                SearchResponse::Pong,
                SearchResponse::Error("Volume not ready".into()),
                SearchResponse::Error("Server busy, try again later".into()),
                SearchResponse::Error("Path not found in index".into()),
            ] {
                assert_eq!(classify(kind, &response), Outcome::Inconclusive);
            }
        }
    }

    #[test]
    fn successful_empty_responses_are_failures_not_authorization_denials() {
        for (kind, response) in accepted_responses() {
            assert_eq!(classify(kind, &response), Outcome::Accepted);
        }
    }

    #[test]
    fn wrong_endpoint_response_is_not_a_pass() {
        for (kind, _) in accepted_responses() {
            assert_eq!(
                classify(kind, &SearchResponse::WarmStarted),
                Outcome::Inconclusive
            );
        }
    }

    #[test]
    fn all_four_checks_run_once_with_a_healthy_service_and_explicit_denials() {
        let fixture = Fixture::new().unwrap();
        let mut count = 0;
        assert_eq!(
            run(fixture.checks().unwrap(), |request| {
                count += 1;
                Ok(if matches!(request, SearchRequest::Ping) {
                    SearchResponse::Pong
                } else {
                    SearchResponse::Error("Authorization failed".into())
                })
            }),
            Outcome::Pass
        );
        assert_eq!(count, 4);
    }

    #[test]
    fn failed_ping_stops_before_the_index_requests() {
        let fixture = Fixture::new().unwrap();
        let mut count = 0;
        assert_eq!(
            run(fixture.checks().unwrap(), |_| {
                count += 1;
                Err("synthetic unavailable service")
            }),
            Outcome::Inconclusive
        );
        assert_eq!(count, 1);
    }

    #[test]
    fn transport_failure_never_counts_as_denial_and_remaining_checks_still_run() {
        let fixture = Fixture::new().unwrap();
        let mut count = 0;
        let outcome = run(fixture.checks().unwrap(), |request| {
            count += 1;
            match request {
                SearchRequest::Ping => Ok(SearchResponse::Pong),
                SearchRequest::Query { .. } => Err("synthetic transport failure"),
                _ => Ok(SearchResponse::Error("Authorization failed".into())),
            }
        });
        assert_eq!(outcome, Outcome::Inconclusive);
        assert_eq!(count, 4);
    }

    #[test]
    fn accepted_query_remains_failure_despite_a_later_inconclusive_error() {
        let fixture = Fixture::new().unwrap();
        let outcome = run(fixture.checks().unwrap(), |request| match request {
            SearchRequest::Ping => Ok(SearchResponse::Pong),
            SearchRequest::Query { .. } => Ok(SearchResponse::Results {
                items: vec![],
                has_more: false,
                total_matches: Some(0),
            }),
            SearchRequest::FolderSize { .. } => Err("synthetic unavailable volume"),
            _ => Ok(SearchResponse::Error("Authorization failed".into())),
        });
        assert_eq!(outcome, Outcome::Accepted);
    }

    #[test]
    fn outcome_combination_never_hides_acceptance_or_missing_evidence() {
        assert_eq!(
            Outcome::Pass.combine(Outcome::Inconclusive),
            Outcome::Inconclusive
        );
        assert_eq!(
            Outcome::Inconclusive.combine(Outcome::Pass),
            Outcome::Inconclusive
        );
        assert_eq!(
            Outcome::Accepted.combine(Outcome::Inconclusive),
            Outcome::Accepted
        );
        assert_eq!(
            Outcome::Inconclusive.combine(Outcome::Accepted),
            Outcome::Accepted
        );
    }
}
