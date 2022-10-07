// TODO: punctuation components
#![allow(dead_code)]

use std::collections::VecDeque;
use std::fmt::Display;
use std::ops::Range;

use swc_ecma_visit::swc_ecma_ast as ast;

use crate::swc::convert_span;

#[derive(Debug)]
pub(crate) struct SyntaxToken;

/// An abstract scope name which can consist of multiple [`NameComponent`]s.
#[derive(Debug)]
pub struct ScopeName {
    pub(crate) components: VecDeque<NameComponent>,
}

impl ScopeName {
    pub(crate) fn new() -> Self {
        Self {
            components: Default::default(),
        }
    }

    /// An Iterator over the individual components of this scope name.
    pub fn components(&self) -> impl Iterator<Item = &NameComponent> + '_ {
        self.components.iter()
    }
}

impl Display for ScopeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for c in self.components() {
            f.write_str(c.text())?;
        }
        Ok(())
    }
}

/// An individual component of a [`ScopeName`].
#[derive(Debug)]
pub struct NameComponent {
    pub(crate) inner: NameComponentInner,
}

impl NameComponent {
    /// The source text of this component.
    pub fn text(&self) -> &str {
        match &self.inner {
            NameComponentInner::Interpolation(s) => s,
            NameComponentInner::SourceIdentifierToken(t) => &t.sym,
            NameComponentInner::SourcePunctuationToken(_) => "",
        }
    }

    /// The range of this component inside of the source text.
    ///
    /// This will return `None` for synthetic components that do not correspond
    /// to a specific token inside the source text.
    pub fn range(&self) -> Option<Range<u32>> {
        match &self.inner {
            NameComponentInner::SourceIdentifierToken(t) => Some(convert_span(t.span)),
            NameComponentInner::SourcePunctuationToken(_t) => {
                None
                //Some(convert_text_range(t.text_range()))
            }
            _ => None,
        }
    }

    pub(crate) fn interp(s: &'static str) -> Self {
        Self {
            inner: NameComponentInner::Interpolation(s),
        }
    }
    pub(crate) fn ident(ident: ast::Ident) -> Self {
        Self {
            inner: NameComponentInner::SourceIdentifierToken(ident),
        }
    }
    pub(crate) fn punct(token: SyntaxToken) -> Self {
        Self {
            inner: NameComponentInner::SourcePunctuationToken(token),
        }
    }
}

#[derive(Debug)]
pub(crate) enum NameComponentInner {
    Interpolation(&'static str),
    SourceIdentifierToken(ast::Ident),
    SourcePunctuationToken(SyntaxToken),
}
