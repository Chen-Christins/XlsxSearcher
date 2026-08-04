use crate::db::{self, DbPool};
use crate::types::{AliasStats, AppStateResponse, IndexStatus, JobState, Settings};
use r2d2_sqlite::SqliteConnectionManager;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub pool: DbPool,
    pub fts_available: bool,
    pub version: String,
    pub directory: Mutex<String>,
    pub settings: Mutex<Settings>,
    pub job: Mutex<JobState>,
    pub app_handle: Mutex<Option<tauri::AppHandle>>,
    pub state_file: PathBuf,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        let config = load_config();
        let data_dir = expand_user(config.data_dir.as_deref().unwrap_or("~/.local/XlsxSearcher"));
        fs::create_dir_all(&data_dir).expect("create data dir");
        let db_path = PathBuf::from(&data_dir).join("index.db");
        let state_file = PathBuf::from(&data_dir).join("webui_state.json");

        let manager = SqliteConnectionManager::file(&db_path).with_init(|conn| {
            conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))?;
            conn.execute("PRAGMA synchronous=NORMAL", [])?;
            conn.execute("PRAGMA foreign_keys=ON", [])?;
            Ok(())
        });
        let pool = r2d2::Pool::builder()
            .max_size(4)
            .min_idle(Some(0))
            .build(manager)
            .expect("create sqlite pool");
        let fts_available = db::init_db(&pool);

        let directory = load_directory(&state_file);
        let settings = load_settings(&state_file);
        Arc::new(Self {
            pool,
            fts_available,
            version: config.version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
            directory: Mutex::new(directory),
            settings: Mutex::new(settings),
            job: Mutex::new(JobState::default()),
            app_handle: Mutex::new(None),
            state_file,
        })
    }

    pub fn get_state(&self) -> AppStateResponse {
        let index = db::get_index_status(&self.pool).unwrap_or(IndexStatus {
            file_count: 0,
            sheet_count: 0,
            indexed_cell_sheet_count: 0,
            pending_deep_index_count: 0,
        });
        let aliases = db::get_alias_stats(&self.pool).unwrap_or(AliasStats {
            alias_count: 0,
            mapping_count: 0,
        });
        let job = self.job.lock().expect("job lock").clone();
        AppStateResponse {
            version: self.version.clone(),
            directory: self.directory.lock().expect("directory lock").clone(),
            index,
            aliases,
            job,
            settings: self.settings.lock().expect("settings lock").clone(),
        }
    }

    pub fn set_directory(&self, directory: String) {
        *self.directory.lock().expect("directory lock") = directory;
        self.save_state();
    }

    pub fn set_settings(&self, settings: Settings) {
        *self.settings.lock().expect("settings lock") = settings;
        self.save_state();
    }

    fn save_state(&self) {
        let directory = self.directory.lock().expect("directory lock").clone();
        let settings = self.settings.lock().expect("settings lock").clone();
        if let Ok(json) = serde_json::to_string(&serde_json::json!({
            "directory": directory,
            "settings": settings,
        })) {
            let _ = fs::write(&self.state_file, json);
        }
    }

    pub fn set_progress(&self, current: usize, total: usize) {
        let mut job = self.job.lock().expect("job lock");
        job.current = current;
        job.total = total;
    }

    pub fn finish_job(&self, result: Option<Value>, message: String, error: Option<String>) {
        let mut job = self.job.lock().expect("job lock");
        job.running = false;
        job.result = result;
        job.message = message;
        job.error = error;
    }

    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.lock().expect("app handle lock") = Some(handle);
    }
}

struct Config {
    version: Option<String>,
    data_dir: Option<String>,
}

fn load_config() -> Config {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config_path = manifest_dir.join("../app.yml");
    let Ok(content) = fs::read_to_string(config_path) else {
        return Config {
            version: None,
            data_dir: None,
        };
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&content) else {
        return Config {
            version: None,
            data_dir: None,
        };
    };
    let app = value.get("app");
    Config {
        version: app
            .and_then(|v| v.get("version"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
        data_dir: app
            .and_then(|v| v.get("data_dir"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string()),
    }
}

fn expand_user(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

fn load_directory(state_file: &PathBuf) -> String {
    let Ok(content) = fs::read_to_string(state_file) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return String::new();
    };
    value
        .get("directory")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .filter(|v| std::path::Path::new(v).is_dir())
        .unwrap_or_default()
}

fn load_settings(state_file: &PathBuf) -> Settings {
    let Ok(content) = fs::read_to_string(state_file) else {
        return Settings::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Settings::default();
    };
    value
        .get("settings")
        .and_then(|v| serde_json::from_value::<Settings>(v.clone()).ok())
        .unwrap_or_default()
}
