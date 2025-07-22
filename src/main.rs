use dotenv::dotenv;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env,
    fs::File,
    io::{BufReader, Write},
    path::PathBuf,
    sync::Arc,
};
use teloxide::{prelude::*, utils::command::BotCommands};
use tokio::sync::Mutex;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<MessageRole>,
}

#[derive(Serialize)]
struct MessageRole {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageContent,
}

#[derive(Deserialize)]
struct MessageContent {
    content: String,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
enum Command {
    #[command(description = "Ask something to GPT.")]
    Ask(String),
    #[command(description = "Add user to white list.")]
    AddUser(String),
    #[command(description = "Remove user from white list.")]
    RemoveUser(String),
    #[command(description = "Show authorized users.")]
    ListUsers,
    #[command(description = "Show help")]
    Help,
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    let bot = Bot::from_env();

    let whitelist = Arc::new(Mutex::new(WhiteList::load()));
    let whitelist_clone = Arc::clone(&whitelist);

    let command_handler =
        Update::filter_message().branch(dptree::entry().filter_command::<Command>().endpoint(
            move |bot: Bot, msg: Message, cmd: Command| {
                let whitelist = Arc::clone(&whitelist_clone);

                async move { handle_command(bot, msg, cmd, whitelist).await }
            },
        ));

    // Хендлер свободного текста (не команд)
    let free_text_handler = Update::filter_message().branch(
        dptree::filter_map(|msg: Message| {
            let text = msg.text().unwrap_or("");
            if Command::parse(text, "YourBotName").is_err() {
                Some(msg)
            } else {
                None
            }
        })
        .endpoint(handle_free_text),
    );

    let handler = command_handler.branch(free_text_handler);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![whitelist])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    //Command::repl(bot, answer).await; //teloxide::commands_repl(bot, answer, Command::ty()).await;
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    whitelist: Arc<Mutex<WhiteList>>,
) -> ResponseResult<()> {
    let username = msg.from().and_then(|u| u.username.clone());

    let is_admin = matches!(username.as_deref(), Some("ksander314"));

    let is_allowed = if let Some(u) = username.as_deref() {
        let wl = whitelist.lock().await;
        wl.is_allowed(u)
    } else {
        false
    };

    match cmd {
        Command::RemoveUser(user) if is_admin => {
            let mut wl = whitelist.lock().await;
            if wl.remove_user(&user) {
                bot.send_message(msg.chat.id, format!("🗑 Removed @{user}"))
                    .await?;
            } else {
                bot.send_message(msg.chat.id, format!("⚠️ @{user} was not in whitelist"))
                    .await?;
            }
        }
        Command::AddUser(user) if is_admin => {
            let mut wl = whitelist.lock().await;
            if wl.add_user(&user) {
                bot.send_message(msg.chat.id, format!("✅ Added @{user}"))
                    .await?;
            } else {
                bot.send_message(msg.chat.id, format!("ℹ️ @{user} already in whitelist"))
                    .await?;
            }
        }
        Command::ListUsers if is_admin => {
            let wl = whitelist.lock().await;
            let list = wl.list().join("\n@");
            bot.send_message(msg.chat.id, format!("👥 Whitelisted:\n@{}", list))
                .await?;
        }
        Command::Ask(q) if is_allowed => {
            let reply = ask_gpt(&q)
                .await
                .unwrap_or("Error contacting OpenAI.".to_string());
            bot.send_message(msg.chat.id, reply).await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        _ => {
            bot.send_message(msg.chat.id, "⛔️ Access denied").await?;
        }
    }
    Ok(())
}

async fn handle_free_text(
    bot: Bot,
    msg: Message,
    whitelist: Arc<Mutex<WhiteList>>,
) -> ResponseResult<()> {
    let username = msg.from().and_then(|u| u.username.as_deref());
    let is_allowed = if let Some(u) = username {
        let wl = whitelist.lock().await;
        wl.is_allowed(u)
    } else {
        false
    };

    if !is_allowed {
        bot.send_message(msg.chat.id, "⛔️ Access denied").await?;
        return Ok(());
    };

    if let Some(text) = msg.text() {
        let reply = ask_gpt(text)
            .await
            .unwrap_or("Error contacting OpenAI.".to_string());
        bot.send_message(msg.chat.id, reply).await?;
    }
    Ok(())
}

async fn ask_gpt(prompt: &str) -> Result<String, reqwest::Error> {
    let api_key = env::var("TWM_OPENAI_API_KEY").expect("TWM_OPENAI_API_KEY not set");

    let client = reqwest::Client::new();

    let body = ChatRequest {
        model: "gpt-4.1".to_string(),
        messages: vec![MessageRole {
            role: "user".to_string(),
            content: prompt.to_string(),
        }],
    };

    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;

    let result: ChatResponse = response.json().await?;

    Ok(result.choices[0].message.content.clone())
}

const WHITELIST_FILE: &str = "whitelist.json";
fn get_config_path(file_name: &str) -> PathBuf {
    let config_dir = env::var("TWM_CONFIG_DIR").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(config_dir).join(file_name)
}

#[derive(Serialize, Deserialize, Default)]
struct WhiteList {
    users: HashSet<String>,
}

impl WhiteList {
    fn load() -> Self {
        let path = get_config_path(WHITELIST_FILE);
        if path.exists() {
            let file = File::open(&path).expect("Failed to open whitelist file");
            let reader = BufReader::new(file);
            serde_json::from_reader(reader).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    fn save(&self) {
        let path = get_config_path(WHITELIST_FILE);
        let json = serde_json::to_string_pretty(self).expect("Failed to serialize whitelist");
        let mut file = File::create(path).expect("Failed to create whitelist file");
        file.write_all(json.as_bytes())
            .expect("Failed to write whitelist file");
    }

    fn remove_user(&mut self, username: &str) -> bool {
        let removed = self.users.remove(username);
        if removed {
            self.save();
        }
        removed
    }

    fn add_user(&mut self, username: &str) -> bool {
        let added = self.users.insert(username.to_string());
        if added {
            self.save();
        }
        added
    }

    fn is_allowed(&self, username: &str) -> bool {
        self.users.contains(username)
    }

    fn list(&self) -> Vec<String> {
        self.users.iter().cloned().collect()
    }
}
