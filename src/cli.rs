use crate::app_config::AppConfig;
use crate::database::Database;
use crate::domain::Permission;
use crate::repositories::{UserInsertError, UserRepository};
use clap::{Parser, Subcommand};
use std::collections::BTreeSet;
use std::error::Error;
use std::path::Path;
use strum::IntoEnumIterator;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(version, about)]
pub struct Cli {
    /// Without a command, Hearth runs
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Manage the users of the API
    #[command(subcommand)]
    User(UserCommand),
}

#[derive(Subcommand, Debug)]
pub enum UserCommand {
    /// Create a user
    Create {
        name: String,
        /// Grant all permissions
        #[arg(long)]
        admin: bool,
    },
}

pub async fn run(command: Command, config: &AppConfig) -> Result<(), Box<dyn Error + Send + Sync>> {
    match command {
        Command::User(UserCommand::Create { name, admin }) => {
            let name = validated_name(&name)?;
            let database = Database::open(Path::new(config.db().file_name()))?;
            create_user(&UserRepository::new(database), name, admin).await
        }
    }
}

fn validated_name(name: &str) -> Result<&str, Box<dyn Error + Send + Sync>> {
    let name = name.trim();
    if name.is_empty() {
        return Err("The name can't be empty".into());
    }
    Ok(name)
}

async fn create_user(
    user_repository: &UserRepository,
    name: &str,
    admin: bool,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let permissions: BTreeSet<_> = if admin { Permission::iter().collect() } else { BTreeSet::new() };
    let id = Uuid::now_v7().to_string();
    match user_repository.insert(&id, name, permissions).await {
        Ok(user) => {
            println!("Created user '{}' with {} permission(s)", user.name, user.permissions.len());
            Ok(())
        }
        Err(UserInsertError::NameAlreadyExists) => Err(format!("a user named '{name}' already exists").into()),
        Err(err) => Err(err.into()),
    }
}
