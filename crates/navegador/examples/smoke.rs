use descriptor::Provenance;
use engine_cdp::Chrome;
use navegador::FixtureServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    eprintln!("[smoke] servidor");
    let server = FixtureServer::start();
    eprintln!(
        "[smoke] page={} sink={}",
        server.page_origin(),
        server.sink_origin()
    );

    eprintln!("[smoke] lanzando chromium");
    let mut chrome = Chrome::launch().await?;
    eprintln!("[smoke] chromium puerto {:?}", chrome.port());

    let url = server.page_url("legit.html");
    eprintln!("[smoke] navegando a {url}");
    chrome.navigate(&url).await?;
    eprintln!("[smoke] cargada");

    let descriptor = chrome.extract("#go", 1, Provenance::User).await?;
    eprintln!(
        "[smoke] descriptor: {}",
        serde_json::to_string_pretty(&descriptor)?
    );

    eprintln!("[smoke] despachando");
    let outcome = chrome.dispatch(&descriptor).await?;
    eprintln!("[smoke] resultado: {outcome:?}");
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    eprintln!("[smoke] peticiones: {:?}", server.requests());
    Ok(())
}
