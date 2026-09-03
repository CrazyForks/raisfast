//! Real-LLM smoke demo: native tool calling against an OpenAI-compatible endpoint.
//!
//! Env:
//!   RAISFAST_AI_BASE_URL   (default https://api.openai.com/v1; Ollama: http://localhost:11434/v1)
//!   RAISFAST_AI_API_KEY    (optional, e.g. local Ollama needs none)
//!   RAISFAST_AI_MODEL      (default gpt-4o-mini)
//!
//! Run:
//!   cargo run -p raisfast-agent --example chat
//!   cargo run -p raisfast-agent --example chat -- "今天是几号？12345*9876 是多少？"

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use raisfast_agent::loop_::TurnEvent;
use raisfast_agent::provider::openai::OpenAiCompatProvider;
use raisfast_agent::tool::{Tool, ToolExecution};
use raisfast_agent::{ToolRegistry, TurnConfig, TurnEngine};
use serde_json::Value;

struct TodayTool;

#[async_trait]
impl Tool for TodayTool {
    fn name(&self) -> &str {
        "today"
    }
    fn description(&self) -> &str {
        "Return today's UTC date as YYYY-MM-DD."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    async fn execute(&self, _args: Value) -> ToolExecution {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs() as i64;
        let days = secs.div_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        Ok(format!("{y:04}-{m:02}-{d:02}"))
    }
}

struct CalcTool;

#[async_trait]
impl Tool for CalcTool {
    fn name(&self) -> &str {
        "calculate"
    }
    fn description(&self) -> &str {
        "Evaluate a basic arithmetic expression (+, -, *, /, parentheses)."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "expression": { "type": "string" } },
            "required": ["expression"]
        })
    }
    async fn execute(&self, args: Value) -> ToolExecution {
        let expr = args
            .get("expression")
            .and_then(Value::as_str)
            .ok_or_else(|| "expression (string) required".to_string())?;
        let v = eval(expr)?;
        Ok(format_num(v))
    }
}

fn main() {
    let prompt = std::env::args().nth(1).unwrap_or_else(|| {
        "今天是几号？请把 12345 * 9876 算出来。最后用一句话把两个结果告诉我。".to_string()
    });
    let base = std::env::var("RAISFAST_AI_BASE_URL")
        .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
    let key = std::env::var("RAISFAST_AI_API_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    let model = std::env::var("RAISFAST_AI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());

    if key.is_none() && base.contains("api.openai.com") {
        eprintln!(
            "RAISFAST_AI_API_KEY is not set; set it (or point RAISFAST_AI_BASE_URL at Ollama etc.)."
        );
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let provider: Arc<dyn raisfast_agent::ModelProvider> =
            Arc::new(OpenAiCompatProvider::new(base, key));
        let mut tools = ToolRegistry::new();
        tools.register(TodayTool);
        tools.register(CalcTool);

        let engine = TurnEngine::new(
            provider,
            model,
            Arc::new(tools),
            TurnConfig {
                max_iterations: 8,
                temperature: Some(0.2),
            },
        );

        let mut history = Vec::new();
        let outcome = engine
            .run(
                &mut history,
                Some(
                    "你是能调用工具的助手。需要日期就调 today，需要计算就调 calculate，回答简洁。",
                ),
                &prompt,
            )
            .await;

        match outcome {
            Ok(o) => {
                for e in &o.events {
                    match e {
                        TurnEvent::Text { text } => println!("[text] {text}"),
                        TurnEvent::ToolCall { name, arguments } => {
                            println!("[tool_call] {name} {arguments}")
                        }
                        TurnEvent::ToolResult { name, output } => {
                            println!("[tool_result] {name} -> {output}")
                        }
                    }
                }
                println!("\n--- final ---\n{}", o.text);
                if let Some(u) = o.usage {
                    println!(
                        "\nusage: input={} output={} | iterations={} tools={}",
                        u.input_tokens.unwrap_or(0),
                        u.output_tokens.unwrap_or(0),
                        o.iterations,
                        o.tool_calls_made
                    );
                }
            }
            Err(e) => eprintln!("turn failed: {e}"),
        }
    });
}

// ── date helper (UTC, no chrono dep) ────────────────────────────────────────

/// Convert days since 1970-01-01 to (year, month, day). Standard civil calendar
/// algorithm (Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ── tiny expression evaluator (recursive descent, f64) ─────────────────────

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn skip_ws(&mut self) {
        while let Some(c) = self.chars.get(self.pos) {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.chars.get(self.pos).copied() {
                Some('+') => {
                    self.pos += 1;
                    value += self.parse_term()?;
                }
                Some('-') => {
                    self.pos += 1;
                    value -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_ws();
            match self.chars.get(self.pos).copied() {
                Some('*') => {
                    self.pos += 1;
                    value *= self.parse_factor()?;
                }
                Some('/') => {
                    self.pos += 1;
                    let rhs = self.parse_factor()?;
                    if rhs == 0.0 {
                        return Err("division by zero".into());
                    }
                    value /= rhs;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<f64, String> {
        self.skip_ws();
        let neg = match self.chars.get(self.pos).copied() {
            Some('-') => {
                self.pos += 1;
                true
            }
            Some('+') => {
                self.pos += 1;
                false
            }
            _ => false,
        };
        let v = match self.chars.get(self.pos).copied() {
            Some('(') => {
                self.pos += 1;
                let v = self.parse_expr()?;
                self.skip_ws();
                match self.chars.get(self.pos).copied() {
                    Some(')') => self.pos += 1,
                    _ => return Err("expected ')'".into()),
                }
                v
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.parse_number()?,
            Some(c) => return Err(format!("unexpected '{c}'")),
            None => return Err("unexpected end of expression".into()),
        };
        Ok(if neg { -v } else { v })
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while let Some(c) = self.chars.get(self.pos).copied() {
            if c.is_ascii_digit() || c == '.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>()
            .map_err(|e| format!("bad number '{text}': {e}"))
    }
}

fn eval(expr: &str) -> Result<f64, String> {
    let mut p = Parser {
        chars: expr.chars().collect(),
        pos: 0,
    };
    let v = p.parse_expr()?;
    p.skip_ws();
    if p.pos != p.chars.len() {
        let rest: String = p.chars[p.pos..].iter().collect();
        return Err(format!("unexpected trailing input: {rest}"));
    }
    Ok(v)
}

fn format_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 9.007_199_254_740_992e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}
