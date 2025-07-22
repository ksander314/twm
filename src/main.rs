use dotenv::dotenv;
use serde::{Deserialize, Serialize};
use std::env;
use teloxide::{prelude::*, utils::command::BotCommands};

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
    #[command(description = "Show help")]
    Help,
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    let bot = Bot::from_env();

    let command_handler = Update::filter_message().branch(
        dptree::entry()
            .filter_command::<Command>()
            .endpoint(handle_command),
    );

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
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    //Command::repl(bot, answer).await; //teloxide::commands_repl(bot, answer, Command::ty()).await;
}

async fn handle_command(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    if !is_authorized_user(&msg) {
        bot.send_message(msg.chat.id, "⛔️ Access denied").await?;
        return Ok(());
    }
    match cmd {
        Command::Ask(q) => {
            let reply = ask_gpt(&q)
                .await
                .unwrap_or("Error contacting OpenAI.".to_string());
            bot.send_message(msg.chat.id, reply).await?;
        }
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
    }
    Ok(())
}

async fn handle_free_text(bot: Bot, msg: Message) -> ResponseResult<()> {
    if !is_authorized_user(&msg) {
        bot.send_message(msg.chat.id, "⛔️ Access denied").await?;
        return Ok(());
    }
    if let Some(text) = msg.text() {
        let reply = ask_gpt(text)
            .await
            .unwrap_or("Error contacting OpenAI.".to_string());
        bot.send_message(msg.chat.id, reply).await?;
    }
    Ok(())
}

fn is_authorized_user(msg: &Message) -> bool {
    matches!(
        msg.from().and_then(|u| u.username.as_deref()),
        Some("ksander314") | Some("alnasan")
    )
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
