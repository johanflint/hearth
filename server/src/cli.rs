use crate::app_config::AppConfig;
use crate::database::Database;
use crate::domain::{Permission, Token};
use crate::repositories::{StoredUser, TokenInsertError, TokenRepository, UserInsertError, UserRepository};
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
    /// Manage the API tokens of a user
    #[command(subcommand)]
    Token(TokenCommand),
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

#[derive(Subcommand, Debug)]
pub enum TokenCommand {
    /// Create a token. It is shown once and can't be retrieved later
    Create {
        /// The user the token belongs to
        user: String,
        /// A label for the device using the token, e.g. "Kitchen tablet"
        name: String,
    },
    /// List the tokens of a user
    List { user: String },
    /// Revoke a token. It stops working immediately
    Revoke {
        /// The user the token belongs to
        user: String,
        /// A label for the device using the token, e.g. "Kitchen tablet"
        name: String,
    },
}

pub async fn run(command: Command, config: &AppConfig) -> Result<(), Box<dyn Error + Send + Sync>> {
    match command {
        Command::User(UserCommand::Create { name, admin }) => {
            let name = validated_name(&name)?;
            let database = Database::open(Path::new(config.db().file_name()))?;
            create_user(&UserRepository::new(database), name, admin).await
        }
        Command::Token(command) => {
            let database = Database::open(Path::new(config.db().file_name()))?;
            let users = UserRepository::new(database.clone());
            let tokens = TokenRepository::new(database);
            match command {
                TokenCommand::Create { user, name } => {
                    create_token(&users, &tokens, &user, validated_name(&name)?).await
                }
                TokenCommand::List { user } => list_tokens(&users, &tokens, &user).await,
                TokenCommand::Revoke { user, name } => revoke_token(&users, &tokens, &user, &name).await,
            }
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

async fn create_token(
    users: &UserRepository,
    tokens: &TokenRepository,
    user: &str,
    name: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let user = find_user(users, user).await?;
    let token = Token::generate();
    let id = Uuid::now_v7().to_string();
    match tokens.insert(&id, &user.id, name, &token).await {
        Ok(_) => {
            // The token alone on stdout so it can be captured, the hint on stderr so it isn't
            eprintln!("Created token '{name}' for '{}'. Store it now, it can't be shown again:", user.name);
            println!("{}", token.as_str());
            Ok(())
        }
        Err(TokenInsertError::NameAlreadyExists) => {
            Err(format!("User '{}' already has a token named '{name}'", user.name).into())
        }
        Err(err) => Err(err.into()),
    }
}

async fn list_tokens(
    users: &UserRepository,
    tokens: &TokenRepository,
    user: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let user = find_user(users, user).await?;
    let stored = tokens.list_by_user(&user.id).await?;
    if stored.is_empty() {
        println!("User '{}' has no tokens", user.name);
    }

    for token in stored {
        let last_used = token.last_used_at.map_or("never".to_string(), |at| at.to_rfc3339());
        println!("{}\tcreated {}\tlast used {}", token.name, token.created_at.to_rfc3339(), last_used);
    }

    Ok(())
}

async fn revoke_token(
    users: &UserRepository,
    tokens: &TokenRepository,
    user: &str,
    name: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let user = find_user(users, user).await?;
    if !tokens.delete(&user.id, name.trim()).await? {
        return Err(format!("User '{}' has no token named '{name}'", user.name).into());
    }
    println!("Revoked token '{name}' of '{}'", user.name);
    Ok(())
}

async fn find_user(users: &UserRepository, name: &str) -> Result<StoredUser, Box<dyn Error + Send + Sync>> {
    users.by_name(name.trim()).await?.ok_or_else(|| format!("No user named '{name}'").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    struct Fixture {
        database: Database,
        users: UserRepository,
        tokens: TokenRepository,
    }

    fn fixture() -> Fixture {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        Fixture {
            users: UserRepository::new(database.clone()),
            tokens: TokenRepository::new(database.clone()),
            database,
        }
    }

    async fn count_tokens(database: &Database) -> i64 {
        database
            .call(|conn| {
                conn.query_row("SELECT COUNT(*) FROM api_tokens", [], |row| row.get(0))
                    .map_err(crate::database::DatabaseError::from)
            })
            .await
            .unwrap()
    }

    #[rstest]
    #[case::plain("johan", "johan")]
    #[case::padded("  johan\t", "johan")]
    #[case::inner_space(" Kitchen tablet ", "Kitchen tablet")]
    fn validated_name_trims_the_name(#[case] name: &str, #[case] expected: &str) {
        assert_eq!(validated_name(name).unwrap(), expected);
    }

    #[rstest]
    #[case::empty("")]
    #[case::blank("  \t ")]
    fn validated_name_rejects_an_empty_name(#[case] name: &str) {
        let err = validated_name(name).unwrap_err();

        assert_eq!(err.to_string(), "The name can't be empty");
    }

    #[tokio::test]
    async fn create_user_grants_all_permissions_to_an_admin() {
        let fixture = fixture();

        create_user(&fixture.users, "johan", true).await.unwrap();

        let user = fixture.users.by_name("johan").await.unwrap().unwrap();
        assert_eq!(user.permissions, Permission::iter().collect::<BTreeSet<_>>());
    }

    #[tokio::test]
    async fn create_user_grants_no_permissions_to_a_regular_user() {
        let fixture = fixture();

        create_user(&fixture.users, "johan", false).await.unwrap();

        let user = fixture.users.by_name("johan").await.unwrap().unwrap();
        assert!(user.permissions.is_empty());
    }

    #[rstest]
    #[case::same_case("johan")]
    #[case::other_case("JOHAN")]
    #[tokio::test]
    async fn create_user_fails_for_a_duplicate_name(#[case] name: &str) {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();

        let err = create_user(&fixture.users, name, true).await.unwrap_err();

        assert_eq!(err.to_string(), format!("a user named '{name}' already exists"));
    }

    #[tokio::test]
    async fn create_token_stores_a_token_for_the_user() {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();

        create_token(&fixture.users, &fixture.tokens, "johan", "Kitchen tablet").await.unwrap();

        assert_eq!(count_tokens(&fixture.database).await, 1);
    }

    #[tokio::test]
    async fn create_token_ignores_the_case_and_padding_of_the_user_name() {
        let fixture = fixture();
        create_user(&fixture.users, "Johan", false).await.unwrap();

        create_token(&fixture.users, &fixture.tokens, " JOHAN ", "Phone").await.unwrap();

        assert_eq!(count_tokens(&fixture.database).await, 1);
    }

    #[tokio::test]
    async fn create_token_fails_for_an_unknown_user() {
        let fixture = fixture();

        let err = create_token(&fixture.users, &fixture.tokens, "nobody", "Phone").await.unwrap_err();

        assert_eq!(err.to_string(), "No user named 'nobody'");
        assert_eq!(count_tokens(&fixture.database).await, 0);
    }

    #[tokio::test]
    async fn create_token_fails_for_a_duplicate_name() {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();

        let err = create_token(&fixture.users, &fixture.tokens, "johan", "phone").await.unwrap_err();

        assert_eq!(err.to_string(), "User 'johan' already has a token named 'phone'");
        assert_eq!(count_tokens(&fixture.database).await, 1);
    }

    #[tokio::test]
    async fn list_tokens_succeeds_for_a_user_with_and_without_tokens() {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();
        create_user(&fixture.users, "anna", false).await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();

        assert!(list_tokens(&fixture.users, &fixture.tokens, "johan").await.is_ok());
        assert!(list_tokens(&fixture.users, &fixture.tokens, "anna").await.is_ok());
    }

    #[tokio::test]
    async fn list_tokens_fails_for_an_unknown_user() {
        let fixture = fixture();

        let err = list_tokens(&fixture.users, &fixture.tokens, "nobody").await.unwrap_err();

        assert_eq!(err.to_string(), "No user named 'nobody'");
    }

    #[tokio::test]
    async fn revoke_token_removes_the_token_of_the_user() {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "johan", "Tablet").await.unwrap();

        revoke_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();

        assert_eq!(count_tokens(&fixture.database).await, 1);
        let user = fixture.users.by_name("johan").await.unwrap().unwrap();
        let remaining = fixture.tokens.list_by_user(&user.id).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "Tablet");
    }

    #[tokio::test]
    async fn revoke_token_ignores_the_case_and_padding_of_the_names() {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();

        revoke_token(&fixture.users, &fixture.tokens, "  JOHAN ", "  phone\t").await.unwrap();

        assert_eq!(count_tokens(&fixture.database).await, 0);
    }

    #[tokio::test]
    async fn revoke_token_keeps_the_token_of_another_user_with_the_same_name() {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();
        create_user(&fixture.users, "anna", false).await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "anna", "Phone").await.unwrap();

        revoke_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();

        assert_eq!(count_tokens(&fixture.database).await, 1);
        let anna = fixture.users.by_name("anna").await.unwrap().unwrap();
        assert_eq!(fixture.tokens.list_by_user(&anna.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn revoke_token_fails_for_an_unknown_user() {
        let fixture = fixture();

        let err = revoke_token(&fixture.users, &fixture.tokens, "nobody", "Phone").await.unwrap_err();

        assert_eq!(err.to_string(), "No user named 'nobody'");
    }

    #[tokio::test]
    async fn revoke_token_fails_for_an_unknown_token_and_keeps_the_others() {
        let fixture = fixture();
        create_user(&fixture.users, "johan", false).await.unwrap();
        create_token(&fixture.users, &fixture.tokens, "johan", "Phone").await.unwrap();

        let err = revoke_token(&fixture.users, &fixture.tokens, "johan", "Tablet").await.unwrap_err();

        assert_eq!(err.to_string(), "User 'johan' has no token named 'Tablet'");
        assert_eq!(count_tokens(&fixture.database).await, 1);
    }
}
