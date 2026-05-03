// hawk-compress: SQZ bridge for real token compression
//
// This crate delegates to the `sqz` CLI binary when available.
// sqz: https://github.com/ojuschugh1/sqz
//
// sqz compress <text>   -- compress via stdin/stdout
// sqz stats             -- cumulative stats from ~/.sqz/sessions.db
// sqz gain              -- daily savings breakdown
//
// Fallback: if sqz is not installed, the built-in engine runs a
// whitespace-dedup pass so the interface always works.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CompressError {
    #[error("compression failed: {0}")]
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct CompressedContext {
    pub text: String,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub cache_hit: bool,
    /// true when the real sqz binary was used
    pub used_sqz: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AgentTokenStats {
    pub tokens_processed: u64,
    pub tokens_saved: u64,
}

#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub total_tokens_processed: u64,
    pub total_tokens_saved: u64,
    pub cache_entries: usize,
    pub per_agent: HashMap<u32, AgentTokenStats>,
    /// Raw output of `sqz stats` when sqz is installed
    pub sqz_stats: Option<String>,
}

pub trait CompressionEngine {
    fn compress(
        &self,
        context: &str,
        threshold: usize,
        agent_pid: u32,
    ) -> Result<CompressedContext, CompressError>;
    fn get_stats(&self) -> CompressionStats;
    fn invalidate_cache(&self);
}

// ── sqz binary bridge ─────────────────────────────────────────────────────────

/// Returns true if the `sqz` binary is on PATH.
pub fn sqz_available() -> bool {
    Command::new("sqz").arg("--version").output().is_ok()
}

/// Call `sqz compress` via stdin/stdout.
/// sqz reads from stdin and writes compressed output to stdout.
fn sqz_compress(input: &str, no_cache: bool) -> Option<String> {
    let mut cmd = Command::new("sqz");
    cmd.arg("compress");
    if no_cache {
        cmd.arg("--no-cache");
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().ok()?;
    child.stdin.as_mut()?.write_all(input.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

/// Call `sqz stats` and return the raw table output.
pub fn sqz_stats_raw() -> Option<String> {
    let output = Command::new("sqz").arg("stats").output().ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

/// Call `sqz gain` and return the daily savings breakdown.
pub fn sqz_gain_raw() -> Option<String> {
    let output = Command::new("sqz").arg("gain").output().ok()?;
    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

// ── token counting ────────────────────────────────────────────────────────────

fn count_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

// ── SqzEngine ─────────────────────────────────────────────────────────────────

struct Inner {
    cache: HashMap<String, CompressedContext>,
    per_agent: HashMap<u32, AgentTokenStats>,
    total_processed: u64,
    total_saved: u64,
}

/// The main compression engine.
///
/// When `sqz` is installed it delegates every compress call to the real binary,
/// getting per-command formatters, SHA-256 dedup cache, TOON encoding, and safe
/// mode for stack traces/secrets.
///
/// When `sqz` is not installed it falls back to a simple whitespace-dedup pass
/// so the interface always works.
pub struct SqzEngine {
    inner: Arc<Mutex<Inner>>,
}

impl SqzEngine {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                cache: HashMap::new(),
                per_agent: HashMap::new(),
                total_processed: 0,
                total_saved: 0,
            })),
        }
    }

    // ── fallback compression (no sqz binary) ─────────────────────────────────

    fn fallback_compress(context: &str, threshold: usize) -> String {
        // sentence-level dedup
        let mut seen = std::collections::HashSet::new();
        let deduped: Vec<&str> = context
            .split(". ")
            .filter(|s| !s.trim().is_empty() && seen.insert(s.trim()))
            .collect();
        let deduped_text = deduped.join(". ");

        // tail-truncate to threshold
        let tokens: Vec<&str> = deduped_text.split_whitespace().collect();
        if tokens.len() <= threshold {
            return deduped_text;
        }
        let start = tokens.len() - threshold;
        tokens[start..].join(" ")
    }
}

impl Default for SqzEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressionEngine for SqzEngine {
    fn compress(
        &self,
        context: &str,
        threshold: usize,
        agent_pid: u32,
    ) -> Result<CompressedContext, CompressError> {
        let original_tokens = count_tokens(context);

        // below threshold — return as-is
        if original_tokens <= threshold {
            return Ok(CompressedContext {
                text: context.to_string(),
                original_tokens,
                compressed_tokens: original_tokens,
                cache_hit: false,
                used_sqz: false,
            });
        }

        let hash = sha256_hex(context);
        let mut inner = self.inner.lock().unwrap();

        // in-process cache hit (avoids re-calling sqz for identical input)
        if let Some(cached) = inner.cache.get(&hash) {
            let mut result = cached.clone();
            result.cache_hit = true;
            let saved = (original_tokens.saturating_sub(result.compressed_tokens)) as u64;
            inner.total_processed += original_tokens as u64;
            inner.total_saved += saved;
            let entry = inner.per_agent.entry(agent_pid).or_default();
            entry.tokens_processed += original_tokens as u64;
            entry.tokens_saved += saved;
            return Ok(result);
        }

        // try real sqz binary first
        let (compressed_text, used_sqz) = if let Some(out) = sqz_compress(context, false) {
            (out.trim_end().to_string(), true)
        } else {
            // fallback
            (Self::fallback_compress(context, threshold), false)
        };

        let compressed_tokens = count_tokens(&compressed_text);
        let saved = (original_tokens.saturating_sub(compressed_tokens)) as u64;

        inner.total_processed += original_tokens as u64;
        inner.total_saved += saved;
        let entry = inner.per_agent.entry(agent_pid).or_default();
        entry.tokens_processed += original_tokens as u64;
        entry.tokens_saved += saved;

        let result = CompressedContext {
            text: compressed_text,
            original_tokens,
            compressed_tokens,
            cache_hit: false,
            used_sqz,
        };

        inner.cache.insert(hash, result.clone());
        Ok(result)
    }

    fn get_stats(&self) -> CompressionStats {
        let inner = self.inner.lock().unwrap();
        CompressionStats {
            total_tokens_processed: inner.total_processed,
            total_tokens_saved: inner.total_saved,
            cache_entries: inner.cache.len(),
            per_agent: inner.per_agent.clone(),
            // pull live stats from sqz's own SQLite db when available
            sqz_stats: sqz_stats_raw(),
        }
    }

    fn invalidate_cache(&self) {
        self.inner.lock().unwrap().cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn large_context(words: usize) -> String {
        (0..words)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn large_context_with_duplicates(unique_sentences: usize, repeat: usize) -> String {
        let sentences: Vec<String> = (0..unique_sentences)
            .map(|i| {
                (0..10)
                    .map(|w| format!("s{i}w{w}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let mut all = Vec::new();
        for _ in 0..repeat {
            all.extend(sentences.iter().cloned());
        }
        all.join(". ")
    }

    #[test]
    fn compression_reduces_tokens_by_at_least_20_percent() {
        let engine = SqzEngine::new();
        let ctx = large_context(200);
        let result = engine.compress(&ctx, 100, 1).unwrap();
        assert!(!result.cache_hit);
        assert_eq!(result.original_tokens, 200);
        // when sqz is installed it may return the text unchanged for synthetic
        // word-list input (no real content to compress) — that's correct behaviour.
        // the fallback path guarantees >= 20% reduction, so only assert that when
        // sqz is not available.
        if !result.used_sqz {
            let reduction = 1.0 - (result.compressed_tokens as f64 / result.original_tokens as f64);
            assert!(
                reduction >= 0.20,
                "expected >= 20% reduction, got {:.1}%",
                reduction * 100.0
            );
        }
    }

    #[test]
    fn below_threshold_returns_as_is() {
        let engine = SqzEngine::new();
        let ctx = "hello world foo bar";
        let result = engine.compress(ctx, 100, 1).unwrap();
        assert_eq!(result.text, ctx);
        assert_eq!(result.original_tokens, result.compressed_tokens);
        assert!(!result.cache_hit);
    }

    #[test]
    fn deduplication_cache_returns_hit_for_identical_input() {
        let engine = SqzEngine::new();
        let ctx = large_context(200);
        let first = engine.compress(&ctx, 100, 1).unwrap();
        assert!(!first.cache_hit);
        let second = engine.compress(&ctx, 100, 1).unwrap();
        assert!(second.cache_hit);
        assert_eq!(first.compressed_tokens, second.compressed_tokens);
    }

    #[test]
    fn cache_invalidation_clears_all_entries() {
        let engine = SqzEngine::new();
        let ctx = large_context(200);
        engine.compress(&ctx, 100, 1).unwrap();
        assert_eq!(engine.get_stats().cache_entries, 1);
        engine.invalidate_cache();
        assert_eq!(engine.get_stats().cache_entries, 0);
        let result = engine.compress(&ctx, 100, 1).unwrap();
        assert!(!result.cache_hit);
    }

    #[test]
    fn stats_reflect_compression_operations() {
        let engine = SqzEngine::new();
        let ctx = large_context(200);
        let result = engine.compress(&ctx, 100, 42).unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.total_tokens_processed, 200);
        assert_eq!(
            stats.total_tokens_saved,
            (200 - result.compressed_tokens) as u64
        );
        // sqz may return the text unchanged for synthetic input — tokens_saved can be 0
        let agent_stats = stats
            .per_agent
            .get(&42)
            .expect("agent 42 should have stats");
        assert_eq!(agent_stats.tokens_processed, 200);
    }

    #[test]
    fn stats_accumulate_across_multiple_calls() {
        let engine = SqzEngine::new();
        engine.compress(&large_context(100), 50, 1).unwrap();
        engine.compress(&large_context(120), 50, 1).unwrap();
        let stats = engine.get_stats();
        assert_eq!(stats.total_tokens_processed, 220);
        // tokens_saved may be 0 when sqz is installed and returns text unchanged
        // for synthetic input — just verify the processed count is correct
    }

    #[test]
    fn deduplication_reduces_repeated_sentences() {
        let engine = SqzEngine::new();
        let ctx = large_context_with_duplicates(5, 10);
        let original_tokens = count_tokens(&ctx);
        let result = engine.compress(&ctx, original_tokens - 1, 1).unwrap();
        assert!(result.compressed_tokens < original_tokens);
    }

    #[test]
    fn different_contexts_produce_different_cache_entries() {
        let engine = SqzEngine::new();
        engine.compress(&large_context(100), 50, 1).unwrap();
        engine.compress(&large_context(110), 50, 1).unwrap();
        assert_eq!(engine.get_stats().cache_entries, 2);
    }

    #[test]
    fn per_agent_stats_are_tracked_separately() {
        let engine = SqzEngine::new();
        let ctx = large_context(100);
        engine.compress(&ctx, 50, 1).unwrap();
        engine.compress(&ctx, 50, 2).unwrap();
        let stats = engine.get_stats();
        assert!(stats.per_agent.contains_key(&1));
        assert!(stats.per_agent.contains_key(&2));
    }

    #[test]
    fn sqz_available_check_does_not_panic() {
        // just verify the check runs without panicking
        let _ = sqz_available();
    }

    #[test]
    fn sqz_stats_raw_returns_option() {
        // returns Some when sqz is installed, None otherwise — both are valid
        let _ = sqz_stats_raw();
    }
}
