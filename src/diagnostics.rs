use serde::Serialize;
use sqlx::Row;

use crate::web::REQUIRED_TABLES;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReport {
    pub database: DatabaseDiagnostic,
    pub tools: Vec<ToolDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDiagnostic {
    pub configured: bool,
    pub reachable: bool,
    pub schema_ready: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDiagnostic {
    pub name: String,
    pub available: bool,
    pub version: Option<String>,
    pub message: Option<String>,
}

pub async fn run(
    database_url: Option<String>,
    transcriber_command: Option<String>,
) -> DiagnosticsReport {
    DiagnosticsReport {
        database: database_diagnostic(database_url).await,
        tools: vec![
            command_diagnostic("yt-dlp", &["--version"]),
            command_diagnostic(
                transcriber_command.as_deref().unwrap_or("whisper"),
                &["--help"],
            ),
        ],
    }
}

async fn database_diagnostic(database_url: Option<String>) -> DatabaseDiagnostic {
    let Some(database_url) = database_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("DATABASE_URL").ok())
    else {
        return DatabaseDiagnostic {
            configured: false,
            reachable: false,
            schema_ready: false,
            message: Some("DATABASE_URL is not configured.".to_string()),
        };
    };

    let pool = match crate::db::connect(&database_url).await {
        Ok(pool) => pool,
        Err(error) => {
            return DatabaseDiagnostic {
                configured: true,
                reachable: false,
                schema_ready: false,
                message: Some(error.to_string()),
            }
        }
    };

    match required_tables_exist(&pool).await {
        Ok(true) => DatabaseDiagnostic {
            configured: true,
            reachable: true,
            schema_ready: true,
            message: None,
        },
        Ok(false) => DatabaseDiagnostic {
            configured: true,
            reachable: true,
            schema_ready: false,
            message: Some("Database is reachable but migrations are missing.".to_string()),
        },
        Err(error) => DatabaseDiagnostic {
            configured: true,
            reachable: true,
            schema_ready: false,
            message: Some(error.to_string()),
        },
    }
}

async fn required_tables_exist(pool: &sqlx::PgPool) -> anyhow::Result<bool> {
    for table in REQUIRED_TABLES {
        let row = sqlx::query("SELECT to_regclass($1)::text AS table_name")
            .bind(table)
            .fetch_one(pool)
            .await?;
        let table_name: Option<String> = row.try_get("table_name")?;
        if table_name.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn command_diagnostic(command: &str, args: &[&str]) -> ToolDiagnostic {
    match std::process::Command::new(command).args(args).output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            ToolDiagnostic {
                name: command.to_string(),
                available: true,
                version: [stdout, stderr]
                    .into_iter()
                    .find(|value| !value.is_empty())
                    .map(|value| value.lines().next().unwrap_or_default().to_string()),
                message: None,
            }
        }
        Ok(output) => ToolDiagnostic {
            name: command.to_string(),
            available: false,
            version: None,
            message: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        },
        Err(error) => ToolDiagnostic {
            name: command.to_string(),
            available: false,
            version: None,
            message: Some(error.to_string()),
        },
    }
}
