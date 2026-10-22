use super::*;

/// A download that ends in success, failure, or a vanished thread lands in
/// the matching state.
#[test]
fn poll_takes_the_thread_result() {
    let (tx, rx) = mpsc::channel();
    let mut download = Download::running(Source::Bundle, Arc::default(), rx);
    download.poll();
    assert!(download.is_running(), "no result yet");

    tx.send(Ok(())).unwrap();
    download.poll();
    assert!(!download.is_running());
    assert_eq!(download.error(), None);

    let (tx, rx) = mpsc::channel();
    let mut failed = Download::running(Source::Eou, Arc::default(), rx);
    tx.send(Err("404".to_string())).unwrap();
    failed.poll();
    assert_eq!(failed.error(), Some("404"));

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let mut vanished = Download::running(Source::Eou, Arc::default(), rx);
    drop(tx);
    vanished.poll();
    assert!(vanished.error().is_some());
}

/// The bar's text reads received of total, in decimal megabytes.
#[test]
fn size_text_shows_received_of_total() {
    let progress = Arc::new(Progress::default());
    let (_tx, rx) = mpsc::channel();
    let download = Download::running(Source::Eou, Arc::clone(&progress), rx);
    progress.add(12_345_678);
    assert_eq!(download.size_text(), "12.3 MB");

    progress.set_total(Some(58_226_559));
    assert_eq!(download.size_text(), "12.3 MB of 58.2 MB");
}

/// Only the bundle is needed to open the manager.
#[test]
fn only_the_bundle_is_required() {
    assert!(Source::Bundle.required());
    assert!(!Source::Eou.required());
}

#[test]
fn each_source_downloads_from_its_own_url() {
    let urls = SourceUrls {
        bundle: "https://example.test/bundle.tgz".to_string(),
        eou: "https://mirror.example.test/eou.zip".to_string(),
    };
    assert_eq!(urls.get(Source::Bundle), "https://example.test/bundle.tgz");
    assert_eq!(urls.get(Source::Eou), "https://mirror.example.test/eou.zip");
}
