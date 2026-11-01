use std::time::Duration;

use crate::core::paths::Paths;
use crate::core::state;
use crate::platform;
use crate::{Result, ops};

/// Start the idle/liveness watchdog for a runspace. It is a tiny detached
/// process: it sleeps, checks the state record and activity timestamp, and
/// destroys the runspace when needed.
pub fn spawn(id: &str) -> Result<i32> {
    platform::spawn_watchdog(id)
}

pub async fn run(id: String) -> Result<()> {
    let paths = Paths::discover()?;

    loop {
        if platform::watchdog_should_stop(&id) {
            return Ok(());
        }
        let Some(record) = state::try_load(&paths, &id) else {
            return Ok(());
        };

        if !platform::is_alive(&record) {
            tracing::info!("runspace {id} is no longer alive; cleaning up");
            destroy_blocking(paths.clone(), record).await;
            return Ok(());
        }

        if let Some(timeout) = record.idle_timeout {
            let last = state::last_activity(&paths, &record);
            let idle = state::now_secs().saturating_sub(last);
            if idle >= timeout {
                tracing::info!("runspace {id} idle for {idle}s (limit {timeout}s); destroying");
                destroy_blocking(paths.clone(), record).await;
                return Ok(());
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn destroy_blocking(paths: Paths, record: state::Record) {
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(_lock) = paths.lock() {
            ops::destroy_record(&paths, &record);
            state::prune_runtime_dir(&paths);
        } else {
            ops::destroy_record(&paths, &record);
        }
    })
    .await;
}
