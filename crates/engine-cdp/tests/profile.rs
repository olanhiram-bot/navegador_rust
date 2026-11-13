//! Perfil efímero: cada instancia usa un directorio temporal propio y lo
//! borra al cerrar, sin estado compartido entre sesiones.

use engine_cdp::Chrome;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn el_perfil_es_efimero_y_se_borra_al_cerrar() {
    let first = Chrome::launch().await.expect("Chromium debe lanzarse");
    let first_dir = first.profile_dir().to_path_buf();
    assert!(first_dir.is_dir(), "el perfil debe existir: {first_dir:?}");
    assert!(
        first_dir.starts_with(std::env::temp_dir()),
        "el perfil debe vivir en un temporal: {first_dir:?}"
    );

    drop(first);
    assert!(
        !first_dir.exists(),
        "el perfil debe borrarse al cerrar: {first_dir:?}"
    );

    let second = Chrome::launch().await.expect("Chromium debe lanzarse");
    assert_ne!(
        second.profile_dir(),
        first_dir.as_path(),
        "dos sesiones no pueden compartir perfil"
    );
}
