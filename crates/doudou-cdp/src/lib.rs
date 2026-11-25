//! Chrome discovery, stealth launch, native async WebSocket CDP transport, and Douyin browser
//! session management.

pub mod cdp;
pub mod chrome;
pub mod fake;
pub mod session;

pub use cdp::{
    CapturedNetworkExchange, CdpBrowser, CdpConnection, CdpEvent, CdpPage, CdpPageOps, Cookie,
    DEFAULT_COMMAND_TIMEOUT, EvalWorld, FORBIDDEN_CDP_METHODS, FetchRequest, FetchResponse,
    FetchResponseKind, ISOLATED_WORLD_NAME, KeyEventSpec, KeyModifiers, LoadWait, MediaBlockMode,
    MouseButton, NetworkCaptureBuffer,
};
pub use chrome::{
    BrowserBrand, CHROME_BIN_ENV, ChromeBinary, ChromeLaunchOptions, ChromeProcess, DEFAULT_LANG,
    DEFAULT_STARTUP_TIMEOUT, DEFAULT_WINDOW_SIZE, DevToolsEndpoint, DiscoverySource, LaunchMode,
    build_launch_args, discover_all_chromes, discover_chrome, discover_chrome_with, infer_brand,
    parse_devtools_active_port, parse_pe_fixed_file_info, parse_reg_query_default_value,
    parse_version_string, poll_json_version, probe_chrome_version,
};
pub use fake::{
    FakeBrowserSession, FakeCdpPage, RecordedClick, RecordedEval, RecordedFileUpload, RecordedWheel,
};
pub use session::{
    AuthPolicy, ChromeSession, DOUYIN_IM_ORIGIN, DOUYIN_WEB_ORIGIN, IM_INIT_PATH_MARKER,
    IM_USER_INFO_PATH_MARKER, LoginClassification, LoginStatus, ModeTransition, NavigationResult,
    PreserveUrl, SessionConfig, SessionMode, SessionStatus, TakeoverState, VerificationKind,
};

const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<CdpConnection>();
    assert_send_sync::<CdpPage>();
    assert_send_sync::<FakeCdpPage>();
    assert_send_sync::<NetworkCaptureBuffer>();
    assert_send::<ChromeProcess>();
    assert_send::<CdpBrowser>();
    assert_send::<ChromeSession>();
    assert_send::<FakeBrowserSession>();
};
