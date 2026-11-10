//! How the browser's fetch errors are read (found in #499): only a source's own "HTTP 404" is a
//! missing file. Any other answer, such as a busy server's 502, is a failure, so the reach steps try
//! the next server. Before, the whole error text was searched for "404", stack trace included, so a
//! 502 thrown from column 20404 of the Worker's bundle read as a missing file and the search stopped.
//!
//! The reasons below are the Worker's errors as the page gives them: the message, then the stack.
//!
//! Label: property

use owd_reach::FetchError;
use owd_web::fetch_error;

/// The Worker's error for a busy Overpass server, as read in the web smoke test: the stack names
/// the bundle at column 20404.
const BUSY: &str = "Error: HTTP 502 from http://127.0.0.1:52570/down/api/interpreter?data=%5Bout%3Ajson%5D
    at I (http://127.0.0.1:52569/owd/app-files/assets/worker-Dp5XRTAD.js:1:27346)
    at http://127.0.0.1:52569/owd/app-files/assets/worker-Dp5XRTAD.js:1:20404
    at __wbg_call_939a2607c4484b0b (http://127.0.0.1:52569/owd/app-files/assets/worker-Dp5XRTAD.js:1:20368)";

#[test]
fn a_busy_server_with_404_in_its_stack_is_a_failure_not_a_missing_file() {
    let url = "http://127.0.0.1:52570/down/api/interpreter";
    match fetch_error(url, BUSY) {
        FetchError::Failed { url: u, reason } => {
            assert_eq!(u, url);
            assert!(reason.contains("HTTP 502"), "{reason}");
        }
        other => panic!("a 502 read as {other:?}"),
    }
}

#[test]
fn only_the_sources_own_404_is_a_missing_file() {
    let url = "http://tiles.example/a.tif";
    let missing = "Error: HTTP 404 from http://tiles.example/a.tif\n    at x (worker.js:1:100)";
    assert!(matches!(fetch_error(url, missing), FetchError::NotFound(u) if u == url));
    // A plain string, as a Worker may throw.
    assert!(matches!(
        fetch_error(url, "HTTP 404 from http://tiles.example/a.tif"),
        FetchError::NotFound(_)
    ));
    // 404 in the address, or in a later line, is not the status.
    let in_url = "Error: HTTP 500 from http://tiles.example/x404y.tif";
    assert!(matches!(
        fetch_error(url, in_url),
        FetchError::Failed { .. }
    ));
    let later = "Error: HTTP 503 from http://tiles.example/a.tif\nnote: HTTP 404 elsewhere";
    assert!(matches!(fetch_error(url, later), FetchError::Failed { .. }));
}
