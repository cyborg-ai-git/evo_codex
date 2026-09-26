use super::*;
use codex_model_provider_info::MLX_PROVIDER_ID;
use codex_model_provider_info::built_in_model_providers;

#[tokio::test]
async fn existing_server_is_reused_and_left_running() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)
        .remove(MLX_PROVIDER_ID)
        .unwrap();
    provider.base_url = Some(format!("http://{address}/v1"));
    let runtime = MlxRuntime::default();
    runtime.ensure_ready(&provider).await.unwrap();
    assert!(runtime.child.lock().await.is_none());
    drop(runtime);
    assert!(std::net::TcpStream::connect(address).is_ok());
}

#[test]
fn live_sessions_share_a_single_owned_runtime() {
    let state = crate::shared_state::ModelProviderSharedState::default();
    let first = state.mlx_server("http://127.0.0.1:8080/v1");
    let second = state.mlx_server("http://127.0.0.1:8080/v1");
    let third = state.mlx_server("http://127.0.0.1:8081/v1");
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert!(!std::sync::Arc::ptr_eq(&first, &third));
}
