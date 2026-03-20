mod api;
mod cli;
mod config;
mod constants;
mod models;
mod output;
mod utils;

use std::env;
use std::process::ExitCode;

use clap::Parser;

use api::{
    AuthHandler, CreateProjectRequest, CreateTaskRequest, TickTickClient, UpdateProjectRequest,
    UpdateTaskRequest,
};
use cli::project::ProjectCommands;
use cli::subtask::SubtaskCommands;
use cli::task::TaskCommands;
use cli::{Cli, Commands};
use config::{Config, TokenStorage};
use constants::{ENV_CLIENT_ID, ENV_CLIENT_SECRET};
use models::{ChecklistItemRequest, Priority, Status};
use output::json::{
    JsonResponse, ProjectData, ProjectListData, SubtaskListData, TaskData, TaskListData,
    VersionData,
};
use output::text;
use output::OutputFormat;
use utils::date_parser::parse_date;

/// Application name
const APP_NAME: &str = env!("CARGO_PKG_NAME");
/// Application version
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> ExitCode {
    // Load environment variables from .env file if present
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();

    // Determine output format
    let format = if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };

    // Run the command and handle errors
    let result = run_command(cli.command, format, cli.quiet).await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if !cli.quiet {
                eprintln!("{}", e);
            }
            ExitCode::FAILURE
        }
    }
}

async fn run_command(command: Commands, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
    match command {
        Commands::Init => cmd_init(format, quiet).await,
        Commands::Reset { force } => cmd_reset(force, format, quiet),
        Commands::Version => cmd_version(format, quiet),
        Commands::Project(cmd) => cmd_project(cmd, format, quiet).await,
        Commands::Task(cmd) => cmd_task(cmd, format, quiet).await,
        Commands::Subtask(cmd) => cmd_subtask(cmd, format, quiet).await,
    }
}

/// Initialize OAuth authentication
async fn cmd_init(format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
    // Check if already initialized
    if TokenStorage::exists()? {
        let message =
            "Already authenticated. Use 'tickrs reset' to clear credentials and re-authenticate.";
        if !quiet {
            output_message(format, message, "ALREADY_INITIALIZED")?;
        }
        return Ok(());
    }

    // Load client credentials from environment
    let client_id = env::var(ENV_CLIENT_ID).map_err(|_| {
        anyhow::anyhow!(
            "Missing {} environment variable. Set it to your TickTick OAuth client ID.",
            ENV_CLIENT_ID
        )
    })?;

    let client_secret = env::var(ENV_CLIENT_SECRET).map_err(|_| {
        anyhow::anyhow!(
            "Missing {} environment variable. Set it to your TickTick OAuth client secret.",
            ENV_CLIENT_SECRET
        )
    })?;

    // Create auth handler and get URL first
    let auth = AuthHandler::new(client_id, client_secret);
    let (auth_url, _) = auth.get_auth_url()?;

    if !quiet && format == OutputFormat::Text {
        println!("Opening browser for TickTick authorization...");
        println!();
        println!("If the browser doesn't open, visit this URL:");
        println!("{}", auth_url);
        println!();
    }

    // Run OAuth flow
    let token = auth.run_oauth_flow().await?;

    // Save token
    TokenStorage::save(&token)?;

    // Initialize config
    let config = Config::default();
    config.save()?;

    let message = "Authentication successful";
    if !quiet {
        output_message(format, message, "SUCCESS")?;
    }

    Ok(())
}

/// Reset configuration and clear stored token
fn cmd_reset(force: bool, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
    // Check if anything exists to reset
    let token_exists = TokenStorage::exists()?;
    let config_path = Config::config_path()?;
    let config_exists = config_path.exists();

    if !token_exists && !config_exists {
        let message = "Nothing to reset - no configuration or token found";
        if !quiet {
            output_message(format, message, "NOTHING_TO_RESET")?;
        }
        return Ok(());
    }

    // Confirm unless --force is specified
    if !force && format == OutputFormat::Text {
        println!("This will delete your stored credentials and configuration.");
        println!("You will need to re-authenticate with 'tickrs init'.");
        print!("Continue? [y/N] ");
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Delete token and config
    if token_exists {
        TokenStorage::delete()?;
    }
    if config_exists {
        Config::delete()?;
    }

    let message = "Configuration and credentials cleared";
    if !quiet {
        output_message(format, message, "SUCCESS")?;
    }

    Ok(())
}

/// Display version information
fn cmd_version(format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = VersionData {
                name: APP_NAME.to_string(),
                version: APP_VERSION.to_string(),
            };
            let response = JsonResponse::success(data);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_version(APP_NAME, APP_VERSION));
        }
    }

    Ok(())
}

/// Output a message in the appropriate format
fn output_message(format: OutputFormat, message: &str, code: &str) -> anyhow::Result<()> {
    match format {
        OutputFormat::Json => {
            let response = JsonResponse::success_with_message(serde_json::json!({}), message);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            if code == "SUCCESS" {
                println!("{}", text::format_success(message));
            } else {
                println!("{}", message);
            }
        }
    }
    Ok(())
}

/// Handle project commands
async fn cmd_project(
    cmd: ProjectCommands,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    match cmd {
        ProjectCommands::List => cmd_project_list(format, quiet).await,
        ProjectCommands::Show { id } => cmd_project_show(&id, format, quiet).await,
        ProjectCommands::Use { name_or_id } => cmd_project_use(&name_or_id, format, quiet).await,
        ProjectCommands::Create {
            name,
            color,
            view_mode,
            kind,
        } => cmd_project_create(&name, color, view_mode, kind, format, quiet).await,
        ProjectCommands::Update {
            id,
            name,
            color,
            closed,
        } => cmd_project_update(&id, name, color, closed, format, quiet).await,
        ProjectCommands::Delete { id, force } => {
            cmd_project_delete(&id, force, format, quiet).await
        }
    }
}

/// List all projects
async fn cmd_project_list(format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
    let client = TickTickClient::new()?;
    let projects = client.list_projects().await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = ProjectListData { projects };
            let response = JsonResponse::success(data);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_project_list(&projects));
        }
    }

    Ok(())
}

/// Show project details
async fn cmd_project_show(id: &str, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
    let client = TickTickClient::new()?;
    let project = client.get_project(id).await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = ProjectData { project };
            let response = JsonResponse::success(data);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_project_details(&project));
        }
    }

    Ok(())
}

/// Set default project for commands
async fn cmd_project_use(
    name_or_id: &str,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let client = TickTickClient::new()?;
    let projects = client.list_projects().await?;

    // Find project by name or ID
    let project = projects
        .iter()
        .find(|p| p.id == name_or_id || p.name.eq_ignore_ascii_case(name_or_id))
        .ok_or_else(|| anyhow::anyhow!("Project not found: {}", name_or_id))?;

    // Update config with the project ID
    let mut config = Config::load()?;
    config.default_project_id = Some(project.id.clone());
    config.save()?;

    if quiet {
        return Ok(());
    }

    let message = format!("Default project set to '{}'", project.name);
    match format {
        OutputFormat::Json => {
            let data = ProjectData {
                project: project.clone(),
            };
            let response = JsonResponse::success_with_message(data, &message);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_success(&message));
        }
    }

    Ok(())
}

/// Create a new project
async fn cmd_project_create(
    name: &str,
    color: Option<String>,
    view_mode: Option<String>,
    kind: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let client = TickTickClient::new()?;

    let request = CreateProjectRequest {
        name: name.to_string(),
        color,
        view_mode,
        kind,
    };

    let project = client.create_project(&request).await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = ProjectData { project };
            let response = JsonResponse::success_with_message(data, "Project created successfully");
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!(
                "{}",
                text::format_success_with_id("Project created", &project.id)
            );
        }
    }

    Ok(())
}

/// Update an existing project
async fn cmd_project_update(
    id: &str,
    name: Option<String>,
    color: Option<String>,
    closed: Option<bool>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let client = TickTickClient::new()?;

    let request = UpdateProjectRequest {
        name,
        color,
        closed,
        view_mode: None,
    };

    let project = client.update_project(id, &request).await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = ProjectData { project };
            let response = JsonResponse::success_with_message(data, "Project updated successfully");
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!(
                "{}",
                text::format_success_with_id("Project updated", &project.id)
            );
        }
    }

    Ok(())
}

/// Delete a project
async fn cmd_project_delete(
    id: &str,
    force: bool,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    // Confirm unless --force is specified
    if !force && format == OutputFormat::Text {
        print!("Delete project '{}'? [y/N] ", id);
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    let client = TickTickClient::new()?;
    client.delete_project(id).await?;

    if quiet {
        return Ok(());
    }

    let message = "Project deleted successfully";
    match format {
        OutputFormat::Json => {
            let response = JsonResponse::success_with_message(serde_json::json!({}), message);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_success(message));
        }
    }

    Ok(())
}

/// Handle task commands
async fn cmd_task(cmd: TaskCommands, format: OutputFormat, quiet: bool) -> anyhow::Result<()> {
    match cmd {
        TaskCommands::List {
            project_id,
            project_name,
            priority,
            tag,
            status,
        } => {
            cmd_task_list(
                project_id,
                project_name,
                priority,
                tag,
                status,
                format,
                quiet,
            )
            .await
        }
        TaskCommands::Show {
            id,
            project_id,
            project_name,
        } => cmd_task_show(&id, project_id, project_name, format, quiet).await,
        TaskCommands::Create {
            title,
            project_id,
            project_name,
            content,
            priority,
            tags,
            date,
            start,
            due,
            all_day,
            timezone,
            items,
        } => {
            cmd_task_create(
                &title,
                project_id,
                project_name,
                content,
                priority,
                tags,
                date,
                start,
                due,
                all_day,
                timezone,
                items,
                format,
                quiet,
            )
            .await
        }
        TaskCommands::Update {
            id,
            project_id,
            project_name,
            title,
            content,
            priority,
            tags,
            date,
            start,
            due,
            all_day,
            timezone,
            items,
        } => {
            cmd_task_update(
                &id,
                project_id,
                project_name,
                title,
                content,
                priority,
                tags,
                date,
                start,
                due,
                all_day,
                timezone,
                items,
                format,
                quiet,
            )
            .await
        }
        TaskCommands::Delete {
            id,
            project_id,
            project_name,
            force,
        } => cmd_task_delete(&id, project_id, project_name, force, format, quiet).await,
        TaskCommands::Complete {
            id,
            project_id,
            project_name,
        } => cmd_task_complete(&id, project_id, project_name, format, quiet).await,
        TaskCommands::Uncomplete {
            id,
            project_id,
            project_name,
        } => cmd_task_uncomplete(&id, project_id, project_name, format, quiet).await,
    }
}

/// Resolve project name to ID by looking up all projects
async fn resolve_project_name(name: &str) -> anyhow::Result<String> {
    let client = TickTickClient::new()?;
    let projects = client.list_projects().await?;
    
    // Special case for Inbox - the API requires the full ID with user ID
    if name.eq_ignore_ascii_case("inbox") {
        // Get the full inbox ID from the API
        if let Ok(inbox_project) = client.get_project("inbox").await {
            return Ok(inbox_project.id);
        }
    }
    
    let project = projects
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| anyhow::anyhow!("Project not found: {}", name))?;
    Ok(project.id.clone())
}

/// Get the project ID from argument, name lookup, or config default
async fn get_project_id(
    project_id: Option<String>,
    project_name: Option<String>,
) -> anyhow::Result<String> {
    match (project_id, project_name) {
        (Some(_), Some(_)) => {
            anyhow::bail!("Cannot specify both --project-id and --project-name")
        }
        (Some(id), None) => Ok(id),
        (None, Some(name)) => resolve_project_name(&name).await,
        (None, None) => {
            let config = Config::load()?;
            config.default_project_id.ok_or_else(|| {
                anyhow::anyhow!(
                    "No project specified. Use --project-id, --project-name, or set a default with 'tickrs project use <name>'"
                )
            })
        }
    }
}

/// List tasks in a project
async fn cmd_task_list(
    project_id: Option<String>,
    project_name: Option<String>,
    priority_filter: Option<Priority>,
    tag_filter: Option<String>,
    status_filter: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let project_id = get_project_id(project_id, project_name).await?;
    let client = TickTickClient::new()?;
    let mut tasks = client.list_tasks(&project_id).await?;

    // Apply filters
    if let Some(priority) = priority_filter {
        tasks.retain(|t| t.priority == priority);
    }

    if let Some(ref tag) = tag_filter {
        let tag_lower = tag.to_lowercase();
        tasks.retain(|t| t.tags.iter().any(|tt| tt.to_lowercase() == tag_lower));
    }

    if let Some(ref status) = status_filter {
        let status_lower = status.to_lowercase();
        match status_lower.as_str() {
            "complete" | "completed" | "done" => {
                tasks.retain(|t| t.status == Status::Complete);
            }
            "incomplete" | "pending" | "open" => {
                tasks.retain(|t| t.status == Status::Normal);
            }
            _ => {
                anyhow::bail!(
                    "Invalid status filter: {}. Use 'complete' or 'incomplete'",
                    status
                );
            }
        }
    }

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let count = tasks.len();
            let data = TaskListData { tasks, count };
            let response = JsonResponse::success(data);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_task_list(&tasks));
        }
    }

    Ok(())
}

/// Show task details
async fn cmd_task_show(
    task_id: &str,
    project_id: Option<String>,
    project_name: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let project_id = get_project_id(project_id, project_name).await?;
    let client = TickTickClient::new()?;
    let task = client.get_task(&project_id, task_id).await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = TaskData { task };
            let response = JsonResponse::success(data);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_task_details(&task));
        }
    }

    Ok(())
}

/// Create a new task
#[allow(clippy::too_many_arguments)]
async fn cmd_task_create(
    title: &str,
    project_id: Option<String>,
    project_name: Option<String>,
    content: Option<String>,
    priority: Option<Priority>,
    tags: Option<String>,
    date: Option<String>,
    start: Option<String>,
    due: Option<String>,
    all_day: bool,
    timezone: Option<String>,
    items: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let project_id = get_project_id(project_id, project_name).await?;

    // Parse dates
    let (start_date, due_date) = parse_task_dates(date, start, due)?;

    // Parse tags
    let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());

    // Parse subtasks/items
    let items_vec = items.map(|i| {
        i.split(',')
            .enumerate()
            .map(|(idx, s)| ChecklistItemRequest::new(s.trim()).with_sort_order(idx as i64))
            .collect()
    });

    let request = CreateTaskRequest {
        title: title.to_string(),
        project_id: project_id.clone(),
        content,
        is_all_day: if all_day { Some(true) } else { None },
        start_date,
        due_date,
        priority: priority.map(|p| p.to_api_value()),
        time_zone: timezone,
        tags: tags_vec,
        items: items_vec,
    };

    let client = TickTickClient::new()?;
    let task = client.create_task(&request).await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = TaskData { task };
            let response = JsonResponse::success_with_message(data, "Task created successfully");
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_success_with_id("Task created", &task.id));
        }
    }

    Ok(())
}

/// Update an existing task
#[allow(clippy::too_many_arguments)]
async fn cmd_task_update(
    task_id: &str,
    project_id: Option<String>,
    project_name: Option<String>,
    title: Option<String>,
    content: Option<String>,
    priority: Option<Priority>,
    tags: Option<String>,
    date: Option<String>,
    start: Option<String>,
    due: Option<String>,
    all_day: Option<bool>,
    timezone: Option<String>,
    items: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let target_project_id = get_project_id(project_id, project_name).await
        .map_err(|e| anyhow::anyhow!("Failed to resolve project: {}", e))?;
    
    let config = Config::load()
        .map_err(|e| anyhow::anyhow!("Failed to load config from {:?}: {}", Config::config_path().unwrap_or_default(), e))?;
    
    let client = TickTickClient::new()
        .map_err(|e| anyhow::anyhow!("Failed to create client: {}", e))?;

    // Get the current task - first try inbox (most common), then target project
    let current_task = match client.get_task("inbox", task_id).await {
        Ok(task) => task,
        Err(_) => {
            // Try target project if not in inbox
            match client.get_task(&target_project_id, task_id).await {
                Ok(task) => task,
                Err(_) => {
                    // Try Journal project as fallback
                    client.get_task("6841b08779c7d1fab649e906", task_id).await
                        .map_err(|e| anyhow::anyhow!("Failed to get task: {}", e))?
                }
            }
        }
    };

    let current_project_id = &current_task.project_id;

    // Check if this is just a project move (no other changes)
    let is_project_move_only = title.is_none()
        && content.is_none()
        && priority.is_none()
        && tags.is_none()
        && date.is_none()
        && start.is_none()
        && due.is_none()
        && all_day.is_none()
        && timezone.is_none()
        && items.is_none();

    // If project is changing and no other fields are being updated, use v2 API move
    if target_project_id != *current_project_id && is_project_move_only {
        // Create mutable client for v2 login
        let mut client_for_move = TickTickClient::new()
            .map_err(|e| anyhow::anyhow!("Failed to create client: {}", e))?;
        
        // Set v2 session if token is provided
        if let Some(ref v2_token) = config.v2_token {
            client_for_move.set_v2_token(v2_token);
        } else if let (Some(ref username), Some(ref password)) = (&config.username, &config.password) {
            // Otherwise try to login with username/password
            client_for_move.login_v2(username, password).await
                .map_err(|e| anyhow::anyhow!(
                    "Failed to login to v2 API: {}.\n\n\
                    Alternative: Set v2_token directly in config.toml with a session token\n\
                    extracted from your browser after logging in.", e))?;
        } else {
            return Err(anyhow::anyhow!(
                "Cannot move tasks between projects: v2 API authentication required.\n\
                Either set username and password, or set v2_token in {}:\n\n\
                # Option 1: Login with credentials (may be rate-limited)\n\
                username = \"your@email.com\"\n\
                password = \"your-password\"\n\n\
                # Option 2: Use a pre-obtained session token\n\
                # (extract from browser cookies after logging in to TickTick)\n\
                v2_token = \"your-session-token\"",
                Config::config_path().unwrap_or_default().display()
            ));
        }
        
        client_for_move.move_task(task_id, current_project_id, &target_project_id).await
            .map_err(|e| anyhow::anyhow!("Failed to move task: {}", e))?;

        if quiet {
            return Ok(());
        }

        match format {
            OutputFormat::Json => {
                let data = TaskData { task: current_task };
                let response = JsonResponse::success_with_message(data, "Task moved successfully");
                println!("{}", response.to_json_string());
            }
            OutputFormat::Text => {
                println!("Task moved successfully");
            }
        }
        return Ok(());
    }

    // If project is changing (with other updates), move first then update
    if target_project_id != *current_project_id {
        // Create mutable client for v2 login
        let mut client_for_move = TickTickClient::new()
            .map_err(|e| anyhow::anyhow!("Failed to create client: {}", e))?;
        
        // Set v2 session if token is provided
        if let Some(ref v2_token) = config.v2_token {
            client_for_move.set_v2_token(v2_token);
        } else if let (Some(ref username), Some(ref password)) = (&config.username, &config.password) {
            client_for_move.login_v2(username, password).await
                .map_err(|e| anyhow::anyhow!(
                    "Failed to login to v2 API: {}.\n\n\
                    Alternative: Set v2_token directly in config.toml.", e))?;
        } else {
            return Err(anyhow::anyhow!(
                "Cannot move tasks between projects: v2 API authentication required.\n\
                Set v2_token in {} or use username/password.",
                Config::config_path().unwrap_or_default().display()
            ));
        }
        
        client_for_move.move_task(task_id, current_project_id, &target_project_id).await
            .map_err(|e| anyhow::anyhow!("Failed to move task: {}", e))?;
    }

    // Parse dates
    let (start_date, due_date) = parse_task_dates(date, start, due)?;

    // Parse tags
    let tags_vec = tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());

    // Parse subtasks/items
    let items_vec = items.map(|i| {
        i.split(',')
            .enumerate()
            .map(|(idx, s)| ChecklistItemRequest::new(s.trim()).with_sort_order(idx as i64))
            .collect()
    });

    let request = UpdateTaskRequest {
        id: task_id.to_string(),
        project_id: target_project_id.clone(),
        title,
        content,
        is_all_day: all_day,
        start_date,
        due_date,
        priority: priority.map(|p| p.to_api_value()),
        time_zone: timezone,
        tags: tags_vec,
        status: None,
        items: items_vec,
    };

    let task = client.update_task(task_id, &request).await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = TaskData { task };
            let response = JsonResponse::success_with_message(data, "Task updated successfully");
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_success_with_id("Task updated", &task.id));
        }
    }

    Ok(())
}

/// Delete a task
async fn cmd_task_delete(
    task_id: &str,
    project_id: Option<String>,
    project_name: Option<String>,
    force: bool,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let project_id = get_project_id(project_id, project_name).await?;

    // Confirm unless --force is specified
    if !force && format == OutputFormat::Text {
        print!("Delete task '{}'? [y/N] ", task_id);
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    let client = TickTickClient::new()?;
    client.delete_task(&project_id, task_id).await?;

    if quiet {
        return Ok(());
    }

    let message = "Task deleted successfully";
    match format {
        OutputFormat::Json => {
            let response = JsonResponse::success_with_message(serde_json::json!({}), message);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_success(message));
        }
    }

    Ok(())
}

/// Mark a task as complete
async fn cmd_task_complete(
    task_id: &str,
    project_id: Option<String>,
    project_name: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let project_id = get_project_id(project_id, project_name).await?;

    let client = TickTickClient::new()?;
    client.complete_task(&project_id, task_id).await?;

    if quiet {
        return Ok(());
    }

    let message = "Task marked as complete";
    match format {
        OutputFormat::Json => {
            let response = JsonResponse::success_with_message(serde_json::json!({}), message);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_success(message));
        }
    }

    Ok(())
}

/// Mark a task as incomplete
async fn cmd_task_uncomplete(
    task_id: &str,
    project_id: Option<String>,
    project_name: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let project_id = get_project_id(project_id, project_name).await?;

    let client = TickTickClient::new()?;
    let task = client.uncomplete_task(&project_id, task_id).await?;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let data = TaskData { task };
            let response = JsonResponse::success_with_message(data, "Task marked as incomplete");
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_success("Task marked as incomplete"));
        }
    }

    Ok(())
}

fn parse_task_dates(
    date: Option<String>,
    start: Option<String>,
    due: Option<String>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    if let Some(date_str) = date {
        let dt = parse_date(&date_str)?;
        let formatted = dt.format("%Y-%m-%dT%H:%M:%S%z").to_string();
        return Ok((Some(formatted.clone()), Some(formatted)));
    }

    let start_date = start
        .map(|s| parse_date(&s).map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%z").to_string()))
        .transpose()?;

    let due_date = due
        .map(|s| parse_date(&s).map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%z").to_string()))
        .transpose()?;

    Ok((start_date, due_date))
}

/// Handle subtask commands
async fn cmd_subtask(
    cmd: SubtaskCommands,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    match cmd {
        SubtaskCommands::List {
            task_id,
            project_id,
            project_name,
        } => cmd_subtask_list(&task_id, project_id, project_name, format, quiet).await,
    }
}

/// List subtasks (checklist items) for a task
async fn cmd_subtask_list(
    task_id: &str,
    project_id: Option<String>,
    project_name: Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> anyhow::Result<()> {
    let project_id = get_project_id(project_id, project_name).await?;
    let client = TickTickClient::new()?;
    let task = client.get_task(&project_id, task_id).await?;

    let subtasks = task.items;

    if quiet {
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let count = subtasks.len();
            let data = SubtaskListData { subtasks, count };
            let response = JsonResponse::success(data);
            println!("{}", response.to_json_string());
        }
        OutputFormat::Text => {
            println!("{}", text::format_subtask_list(&subtasks));
        }
    }

    Ok(())
}
