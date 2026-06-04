use codex_app_server::in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY;
use codex_app_server_sdk::AppServerBuilder;
use codex_app_server_sdk::InProcessClientStartArgs;
use codex_app_server_sdk::InProcessStartArgs;
use codex_arg0::Arg0DispatchPaths;
use codex_config::CloudConfigBundleLoader;
use codex_config::LoaderOverrides;
use codex_config::NoopThreadConfigLoader;
use codex_core::config::Config;
use codex_core::config::ConfigBuilder;
use codex_exec_server::EnvironmentManager;
use codex_feedback::CodexFeedback;
use codex_protocol::protocol::SessionSource;
use codex_runtime_api::ContextAssemblyObserver;
use codex_runtime_api::ContextAssemblyObserverId;
use codex_runtime_api::ContextAssemblyObserverInput;
use codex_runtime_api::ContextError;
use codex_runtime_api::RuntimeRegistry;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tempfile::TempDir;

struct BuilderObserver;

impl ContextAssemblyObserver for BuilderObserver {
    fn id(&self) -> ContextAssemblyObserverId {
        ContextAssemblyObserverId::new("test.builder_observer")
    }

    async fn observe(&self, _input: ContextAssemblyObserverInput) -> Result<(), ContextError> {
        Ok(())
    }
}

async fn test_config(codex_home: &std::path::Path) -> anyhow::Result<Config> {
    match ConfigBuilder::default()
        .codex_home(codex_home.to_path_buf())
        .build()
        .await
    {
        Ok(config) => Ok(config),
        Err(_) => Ok(Config::load_default_with_cli_overrides_for_codex_home(
            codex_home.to_path_buf(),
            Vec::new(),
        )
        .await?),
    }
}

async fn test_start_args(codex_home: &std::path::Path) -> anyhow::Result<InProcessStartArgs> {
    Ok(InProcessStartArgs {
        arg0_paths: Arg0DispatchPaths::default(),
        config: Arc::new(test_config(codex_home).await?),
        cli_overrides: Vec::new(),
        loader_overrides: LoaderOverrides::default(),
        strict_config: false,
        cloud_config_bundle: CloudConfigBundleLoader::default(),
        thread_config_loader: Arc::new(NoopThreadConfigLoader),
        feedback: CodexFeedback::new(),
        log_db: None,
        state_db: None,
        runtime_registry: RuntimeRegistry::default(),
        environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
        config_warnings: Vec::new(),
        session_source: SessionSource::Cli,
        enable_codex_api_key_env: false,
        initialize: codex_app_server_protocol::InitializeParams {
            client_info: codex_app_server_protocol::ClientInfo {
                name: "codex-app-server-sdk-test".to_string(),
                title: None,
                version: "0.0.0".to_string(),
            },
            capabilities: None,
        },
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
}

async fn test_client_start_args(
    codex_home: &std::path::Path,
) -> anyhow::Result<InProcessClientStartArgs> {
    Ok(InProcessClientStartArgs {
        arg0_paths: Arg0DispatchPaths::default(),
        config: Arc::new(test_config(codex_home).await?),
        cli_overrides: Vec::new(),
        loader_overrides: LoaderOverrides::default(),
        strict_config: false,
        cloud_config_bundle: CloudConfigBundleLoader::default(),
        feedback: CodexFeedback::new(),
        log_db: None,
        state_db: None,
        environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
        config_warnings: Vec::new(),
        session_source: SessionSource::Cli,
        enable_codex_api_key_env: false,
        client_name: "layer2-sdk-test".to_string(),
        client_version: "0.1.0".to_string(),
        experimental_api: true,
        opt_out_notification_methods: vec!["tokenUsage/updated".to_string()],
        channel_capacity: DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
    })
}

#[tokio::test]
async fn builder_installs_custom_runtime_registry_without_changing_startup_args()
-> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let args = test_start_args(codex_home.path()).await?;
    let expected_config = Arc::clone(&args.config);
    let expected_session_source = args.session_source.clone();
    let expected_channel_capacity = args.channel_capacity;

    let mut registry = RuntimeRegistry::builder();
    registry
        .context_assembly_observer(BuilderObserver)
        .expect("builder observer should register");

    let args = AppServerBuilder::new(args)
        .runtime_registry(registry.build())
        .into_in_process_start_args();

    assert!(Arc::ptr_eq(&args.config, &expected_config));
    assert_eq!(args.session_source, expected_session_source);
    assert_eq!(args.channel_capacity, expected_channel_capacity);
    assert_eq!(
        args.runtime_registry.context_assembly_observer_id(),
        Some(ContextAssemblyObserverId::new("test.builder_observer"))
    );

    Ok(())
}

#[tokio::test]
async fn builder_reuses_client_start_args_with_custom_runtime_registry() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let args = test_client_start_args(codex_home.path()).await?;

    let mut registry = RuntimeRegistry::builder();
    registry
        .context_assembly_observer(BuilderObserver)
        .expect("builder observer should register");

    let args = AppServerBuilder::from_client_start_args(args)
        .runtime_registry(registry.build())
        .into_in_process_start_args();

    assert_eq!(args.initialize.client_info.name, "layer2-sdk-test");
    assert_eq!(args.initialize.client_info.version, "0.1.0");
    assert_eq!(
        args.initialize.capabilities,
        Some(codex_app_server_protocol::InitializeCapabilities {
            experimental_api: true,
            request_attestation: false,
            opt_out_notification_methods: Some(vec!["tokenUsage/updated".to_string()]),
        })
    );
    assert_eq!(
        args.runtime_registry.context_assembly_observer_id(),
        Some(ContextAssemblyObserverId::new("test.builder_observer"))
    );

    Ok(())
}
