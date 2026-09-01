use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use captcha::{
    Captcha,
    filters::{Dots, Noise, Wave},
};
use moka::sync::Cache;
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

const CAPTCHA_LENGTH: u32 = 5;
const CAPTCHA_TTL_SECONDS: u64 = 5 * 60;
const CAPTCHA_MAX_CAPACITY: u64 = 10_000;
const CAPTCHA_MAX_ANSWER_BYTES: usize = 16;
const CAPTCHA_NOISE_PROBABILITY: f32 = 0.08;
const CAPTCHA_WAVE_FREQUENCY: f64 = 1.5;
const CAPTCHA_WAVE_AMPLITUDE: f64 = 5.0;
const CAPTCHA_DOT_COUNT: u32 = 4;
const CAPTCHA_DOT_MIN_RADIUS: u32 = 2;
const CAPTCHA_DOT_MAX_RADIUS: u32 = 4;
const CAPTCHA_CHARSET: &[char] = &[
    '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'M', 'N',
    'P', 'Q', 'R', 'T', 'U', 'W', 'X', 'Y',
];

#[derive(Clone)]
pub struct CaptchaService {
    challenges: Cache<Uuid, String>,
}

#[derive(Debug, Serialize)]
pub struct CaptchaChallenge {
    pub id: Uuid,
    pub image_base64: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Error)]
pub enum CaptchaError {
    #[error("CAPTCHA image generation returned no PNG data")]
    ImageGeneration,
    #[error("CAPTCHA image generation task failed")]
    GenerationTask(#[source] tokio::task::JoinError),
}

impl CaptchaService {
    pub fn new() -> Self {
        let challenges = Cache::builder()
            .name("order_captcha_challenges")
            .max_capacity(CAPTCHA_MAX_CAPACITY)
            .time_to_live(Duration::from_secs(CAPTCHA_TTL_SECONDS))
            .build();
        tracing::info!(
            captcha_length = CAPTCHA_LENGTH,
            ttl_seconds = CAPTCHA_TTL_SECONDS,
            max_capacity = CAPTCHA_MAX_CAPACITY,
            noise_probability = CAPTCHA_NOISE_PROBABILITY,
            wave_frequency = CAPTCHA_WAVE_FREQUENCY,
            wave_amplitude = CAPTCHA_WAVE_AMPLITUDE,
            dot_count = CAPTCHA_DOT_COUNT,
            "order CAPTCHA service initialized"
        );
        Self { challenges }
    }

    pub async fn issue(&self) -> Result<CaptchaChallenge, CaptchaError> {
        // PNG 编码属于 CPU 密集型同步工作，交给阻塞线程池，避免并发刷新验证码时占用
        // Axum/Tokio 的异步执行线程。
        let (answer, image_base64) = tokio::task::spawn_blocking(generate_captcha_image)
            .await
            .map_err(CaptchaError::GenerationTask)??;
        let id = Uuid::new_v4();
        self.challenges.insert(id, answer);
        tracing::info!(
            captcha_id = %id,
            expires_in_seconds = CAPTCHA_TTL_SECONDS,
            "order CAPTCHA challenge issued"
        );
        Ok(CaptchaChallenge {
            id,
            image_base64,
            expires_in_seconds: CAPTCHA_TTL_SECONDS,
        })
    }

    pub fn verify_once(&self, id: Uuid, submitted_answer: &str) -> bool {
        // remove 是原子的一次性读取：无论答案正确与否，当前挑战都会失效，并发重复提交
        // 也不可能让同一个验证码通过两次。
        let Some(expected_answer) = self.challenges.remove(&id) else {
            tracing::warn!(captcha_id = %id, "order CAPTCHA rejected: challenge missing or expired");
            return false;
        };
        if submitted_answer.len() > CAPTCHA_MAX_ANSWER_BYTES {
            tracing::warn!(
                captcha_id = %id,
                submitted_answer_bytes = submitted_answer.len(),
                "order CAPTCHA rejected: answer is too long"
            );
            return false;
        }
        let accepted = submitted_answer.trim().to_ascii_uppercase() == expected_answer;
        if accepted {
            tracing::info!(captcha_id = %id, "order CAPTCHA accepted and consumed");
        } else {
            tracing::warn!(captcha_id = %id, "order CAPTCHA rejected: answer mismatch");
        }
        accepted
    }
}

fn generate_captcha_image() -> Result<(String, String), CaptchaError> {
    let mut captcha = Captcha::new();
    captcha
        .set_chars(CAPTCHA_CHARSET)
        .add_chars(CAPTCHA_LENGTH)
        .apply_filter(Noise::new(CAPTCHA_NOISE_PROBABILITY))
        .apply_filter(Wave::new(CAPTCHA_WAVE_FREQUENCY, CAPTCHA_WAVE_AMPLITUDE).horizontal())
        .view(220, 90)
        .apply_filter(
            Dots::new(CAPTCHA_DOT_COUNT)
                .min_radius(CAPTCHA_DOT_MIN_RADIUS)
                .max_radius(CAPTCHA_DOT_MAX_RADIUS),
        );

    let answer = captcha.chars_as_string().to_ascii_uppercase();
    let png = captcha.as_png().ok_or(CaptchaError::ImageGeneration)?;
    Ok((answer, BASE64_STANDARD.encode(png)))
}
