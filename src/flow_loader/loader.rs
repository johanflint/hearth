use crate::extensions::path_ext::FileName;
use crate::flow_engine::flow::Flow;
use crate::flow_loader::factory::{from_json, FlowFactoryError};
use futures::stream::FuturesUnordered;
use std::io;
use std::path::PathBuf;
use thiserror::Error;
use tokio::task::JoinError;
use tokio::{fs, task};
use tokio_stream::wrappers::ReadDirStream;
use tokio_stream::StreamExt;
use tracing::{error, info, instrument, warn};

#[instrument]
pub async fn load_flows_from(directory: &str, extension: &str) -> Result<Vec<Flow>, LoaderError> {
    info!("📁 Loading flows...");
    let files = list_files(directory, extension)
        .await
        .map_err(|e| LoaderError::Io { source: e, path: None })?;

    let results = load_files(files).await;
    let (flows, errors): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);

    for error in errors.iter().filter_map(|res| res.as_ref().err()) {
        log_error(error);
    }

    info!("📁 Loading flows... OK, {} loaded, {} failed", flows.len(), errors.len());
    Ok(flows.into_iter().filter_map(Result::ok).collect())
}

#[instrument]
async fn list_files(directory: &str, extension: &str) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let dir = fs::read_dir(directory).await?;
    let mut entries = ReadDirStream::new(dir);

    while let Some(entry) = entries.next().await {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some(extension) {
                    files.push(path);
                }
            }
            Err(err) => warn!("⚠️ Unable to read directory entry: {}", err),
        }
    }

    Ok(files)
}

#[instrument(skip_all)]
async fn load_files(paths: Vec<PathBuf>) -> Vec<Result<Flow, LoaderError>> {
    FuturesUnordered::from_iter(paths.into_iter().map(|path| async move {
        match fs::read_to_string(&path).await {
            Ok(content) => task::spawn_blocking(move || from_json(&content).map_err(|e| LoaderError::FlowFactory { source: e, path })).await?,
            Err(err) => Err(LoaderError::Io {
                source: err,
                path: Some(path),
            }),
        }
    }))
    .collect()
    .await
}

#[instrument(skip_all)]
fn log_error(error: &LoaderError) {
    match error {
        LoaderError::FlowFactory { source, path } => warn!("⚠️ Failed to load '{}': {}", path.string_file_name(), source),
        LoaderError::Io { source, path } => match path {
            Some(path) => warn!("⚠️ Failed to load '{}': {}", path.string_file_name(), source),
            None => warn!("⚠️ {}", source),
        },
        LoaderError::JoinError(err) => warn!("⚠️ {}", err),
    }
}

#[derive(Error, Debug)]
pub enum LoaderError {
    #[error("{}", source)]
    FlowFactory { source: FlowFactoryError, path: PathBuf },
    #[error("{}", source)]
    Io { source: io::Error, path: Option<PathBuf> },
    #[error(transparent)]
    JoinError(#[from] JoinError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use test_log::test;

    #[tokio::test]
    async fn list_files_returns_all_relevant_files() -> io::Result<()> {
        let temp_dir = temp_dir();

        let file1 = temp_dir.join("flow.json");
        let file2 = temp_dir.join("no_flow.txt");
        let file3 = temp_dir.join("flow2.json");

        fs::write(&file1, "{}").await?;
        fs::write(&file2, "text").await?;
        fs::write(&file3, "{}").await?;

        let mut files = list_files(temp_dir.to_string_lossy().as_ref(), "json").await?;
        files.sort();
        let string_file_names = files.iter().map(|e| e.to_string_lossy()).collect::<Vec<_>>();

        assert_eq!(
            string_file_names,
            vec![file1.to_string_lossy().into_owned(), file3.to_string_lossy().into_owned(),]
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn load_files_returns_a_flow_for_a_valid_flow_file() -> Result<(), LoaderError> {
        let path = PathBuf::from(format!("{}/tests/resources/flows/logFlow.json", env!("CARGO_MANIFEST_DIR")));
        assert!(path.is_file(), "expected path to be a file");

        let result = load_files(vec![path]).await;
        assert_eq!(result.len(), 1);
        match &result[0] {
            Ok(flow) => assert_eq!(flow.name(), "logFlow"),
            Err(err) => assert!(false, "Expected a flow, found {:?}", err),
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn load_files_returns_an_error_for_an_invalid_flow_file() -> Result<(), LoaderError> {
        let path = PathBuf::from(format!(
            "{}/tests/resources/flows/invalid/missingEndNodeFlow.json",
            env!("CARGO_MANIFEST_DIR")
        ));
        assert!(path.is_file(), "expected path to be a file");

        let result = load_files(vec![path]).await;
        assert_eq!(result.len(), 1);
        match &result[0] {
            Err(err) => assert!(matches!(
                err,
                LoaderError::FlowFactory {
                    source: FlowFactoryError::MissingEndNode,
                    path: _
                }
            )),
            _ => assert!(false, "Expected a FlowFactoryError::MissingEndNode"),
        }

        Ok(())
    }
}
