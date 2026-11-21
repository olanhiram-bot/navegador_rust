use descriptor::Provenance;
use engine_cdp::Chrome;

#[tokio::test]
async fn un_campo_llamado_action_conserva_el_destino_del_formulario() {
    let mut chrome = Chrome::launch().await.unwrap();
    chrome.navigate("data:text/html,<form action='https://example.org/buscar'><input name='action' value='buscar'><input name='elements' value='lista'><button aria-label=Buscar></button></form>").await.unwrap();
    let action = chrome.extract("button", 1, Provenance::User).await.unwrap();
    assert_eq!(
        action.action_url.as_deref(),
        Some("https://example.org/buscar")
    );
    assert_eq!(action.target.name, "Buscar");
    assert_eq!(action.fields["action"], "buscar");
    assert_eq!(action.fields["elements"], "lista");
}
