//! [`ContextRegistry`]: the mention providers, and the one place a mention is resolved.

use std::collections::BTreeMap;
use std::sync::Arc;

use oxikube_domain::agent::ContextBlock;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ContextProviderPort, ContextScope, Mention, MentionPrefix};
use parking_lot::RwLock;

/// Why a provider was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterProviderError {
    /// The provider's prefix is not a valid [`MentionPrefix`].
    #[error("provider prefix {0:?} is not valid: {1}")]
    InvalidPrefix(String, String),
    /// Another provider owns the prefix.
    #[error("another provider already owns @{0}")]
    Duplicate(String),
}

/// Routes `@`-mentions to the provider that owns their prefix (`@logs` to the log provider) and
/// keeps what one resolution returns within the caller's byte budget. One per app: every feature
/// registers its provider as it lands, and the agent panel resolves through it.
#[derive(Default)]
pub struct ContextRegistry {
    providers: RwLock<BTreeMap<String, Arc<dyn ContextProviderPort>>>,
}

impl ContextRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `provider` under its [`mention_prefix`](ContextProviderPort::mention_prefix).
    ///
    /// # Errors
    ///
    /// [`RegisterProviderError`] for an invalid prefix, or one another provider already owns.
    pub fn register(
        &self,
        provider: Arc<dyn ContextProviderPort>,
    ) -> Result<(), RegisterProviderError> {
        let prefix = provider.mention_prefix().to_owned();
        MentionPrefix::new(&prefix).map_err(|e| {
            RegisterProviderError::InvalidPrefix(prefix.clone(), e.message().to_owned())
        })?;
        let mut providers = self.providers.write();
        if providers.contains_key(&prefix) {
            return Err(RegisterProviderError::Duplicate(prefix));
        }
        providers.insert(prefix, provider);
        Ok(())
    }

    /// The registered prefixes, sorted (what the agent panel completes `@` with).
    pub fn prefixes(&self) -> Vec<String> {
        self.providers.read().keys().cloned().collect()
    }

    /// The provider that owns `mention`'s prefix and accepts it.
    pub fn provider_for(&self, mention: &Mention) -> Option<Arc<dyn ContextProviderPort>> {
        let provider = self
            .providers
            .read()
            .get(mention.prefix().as_str())?
            .clone();
        provider.handles(mention).then_some(provider)
    }

    /// Resolves `mention` with its provider. The blocks together hold at most
    /// `scope.max_total_bytes` of body: later blocks are dropped and the one that crosses the limit
    /// is cut, each flagged [`truncated`](ContextBlock::truncated).
    ///
    /// # Errors
    ///
    /// A validation error for a prefix no provider owns, else the provider's own.
    pub async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        let provider = self.provider_for(mention).ok_or_else(|| {
            OxiError::validation(format!(
                "no context provider for {mention}: known mentions are {}",
                self.prefixes()
                    .iter()
                    .map(|p| format!("@{p}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let blocks = provider.resolve(mention, scope).await?;
        Ok(fit_total(blocks, scope.max_total_bytes))
    }

    /// [`resolve`](Self::resolve) for a mention as typed (`@logs/default/web-0`).
    ///
    /// # Errors
    ///
    /// A validation error when `text` is not a mention, else those of [`resolve`](Self::resolve).
    pub async fn resolve_text(
        &self,
        text: &str,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        self.resolve(&Mention::parse(text)?, scope).await
    }
}

impl std::fmt::Debug for ContextRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextRegistry")
            .field("prefixes", &self.prefixes())
            .finish()
    }
}

/// Keeps `blocks` within `max` bytes of body in total.
fn fit_total(blocks: Vec<ContextBlock>, max: usize) -> Vec<ContextBlock> {
    let mut left = max;
    let mut out = Vec::with_capacity(blocks.len());
    for mut block in blocks {
        if left == 0 {
            break;
        }
        if block.body.len() > left {
            block = ContextBlock::bounded(
                block.title,
                block.mime,
                &block.body[..block.body.floor_char_boundary(left)],
            );
            block.truncated = true;
        }
        left -= block.body.len();
        out.push(block);
    }
    out
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use futures::executor::block_on;
    use oxikube_domain::ErrorKind;

    use super::*;

    struct Fixed(&'static str, Vec<ContextBlock>);

    #[async_trait]
    impl ContextProviderPort for Fixed {
        fn mention_prefix(&self) -> &str {
            self.0
        }
        async fn resolve(&self, _: &Mention, _: &ContextScope) -> OxiResult<Vec<ContextBlock>> {
            Ok(self.1.clone())
        }
    }

    #[test]
    fn a_mention_goes_to_the_provider_that_owns_its_prefix() {
        let registry = ContextRegistry::new();
        registry
            .register(Arc::new(Fixed(
                "logs",
                vec![ContextBlock::text("l", "log text")],
            )))
            .unwrap();
        registry
            .register(Arc::new(Fixed(
                "pod",
                vec![ContextBlock::text("p", "pod text")],
            )))
            .unwrap();
        assert_eq!(registry.prefixes(), ["logs", "pod"]);
        let blocks =
            block_on(registry.resolve_text("@logs/default/web-0", &ContextScope::new())).unwrap();
        assert_eq!(blocks[0].body, "log text");
    }

    #[test]
    fn an_unknown_prefix_names_the_known_ones() {
        let registry = ContextRegistry::new();
        registry.register(Arc::new(Fixed("logs", vec![]))).unwrap();
        let err =
            block_on(registry.resolve_text("@events/default", &ContextScope::new())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
        assert!(err.message().contains("@logs"), "{}", err.message());
        let err =
            block_on(registry.resolve_text("logs/default", &ContextScope::new())).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
    }

    #[test]
    fn a_prefix_has_one_owner_and_must_be_valid() {
        let registry = ContextRegistry::new();
        registry.register(Arc::new(Fixed("logs", vec![]))).unwrap();
        assert_eq!(
            registry.register(Arc::new(Fixed("logs", vec![]))),
            Err(RegisterProviderError::Duplicate("logs".into()))
        );
        assert!(matches!(
            registry.register(Arc::new(Fixed("Bad Prefix", vec![]))),
            Err(RegisterProviderError::InvalidPrefix(..))
        ));
    }

    #[test]
    fn the_total_budget_cuts_and_flags_blocks() {
        let registry = ContextRegistry::new();
        registry
            .register(Arc::new(Fixed(
                "logs",
                vec![
                    ContextBlock::text("a", "x".repeat(60)),
                    ContextBlock::text("b", "y".repeat(60)),
                    ContextBlock::text("c", "z".repeat(60)),
                ],
            )))
            .unwrap();
        let scope = ContextScope::new().with_max_total_bytes(100);
        let blocks = block_on(registry.resolve_text("@logs/a/b", &scope)).unwrap();
        assert_eq!(blocks.len(), 2, "the third block is dropped");
        assert_eq!(blocks[0].body.len(), 60);
        assert!(!blocks[0].truncated);
        assert_eq!(blocks[1].body.len(), 40);
        assert!(blocks[1].truncated);
    }
}
