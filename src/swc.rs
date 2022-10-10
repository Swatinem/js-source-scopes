use std::ops::Range;

use swc_common::{BytePos, Span};
use swc_ecma_parser::{Parser, StringInput};
use swc_ecma_visit::swc_ecma_ast as ast;
use swc_ecma_visit::{AstNodePath, VisitAstPath, VisitWithPath};

use crate::scope_name::{NameComponent, ScopeName};
use crate::Scopes;

// TODO:
// - getters / setters
// - maybe even computed properties?

pub fn parse_with_swc(src: &str) -> Scopes {
    let syntax = tracing::trace_span!("parsing source").in_scope(|| {
        let input = StringInput::new(src, BytePos(0), BytePos(src.len() as u32));

        let mut parser = Parser::new(swc_ecma_parser::Syntax::default(), input, None);

        parser.parse_module()
    });
    let syntax = match syntax {
        Ok(syntax) => syntax,
        Err(_) => return vec![],
    };

    // dbg!(&syntax);

    tracing::trace_span!("extracting scopes").in_scope(|| {
        let mut collector = ScopeCollector::new();

        syntax.visit_children_with_path(&mut collector, &mut Default::default());

        collector.into_scopes()
    })
}

/// Converts a [`Span`] into a standard [`Range`].
pub(crate) fn convert_span(span: Span) -> Range<u32> {
    span.lo.0..span.hi.0
}

/// The ScopeCollector is responsible for collection function scopes and computing their names.
///
/// SWCs AST is based around the Visitor pattern. In this case our visitor has some
/// method that act on different function-like AST nodes that we are interested in.
/// From there, the node either has a name itself, or we infer its name from the
/// "path" of parents.
/// As a concrete example:
/// `const name = () => {};`
/// 1. The visitors `visit_arrow_expr` function is invoked for the arrow function
///    on the right hand side. Arrow functions by definition do not have a name.
/// 2. We use the "path" to walk up to the VariableDeclarator.
/// 3. That declarator has a binding pattern on the left hand side, which we use
///    to infer the `name` for the anonymous arrow function expression.
struct ScopeCollector {
    scopes: Scopes,
}

impl ScopeCollector {
    fn new() -> Self {
        Self { scopes: vec![] }
    }

    fn into_scopes(self) -> Scopes {
        self.scopes
    }
}

use swc_ecma_visit::AstParentNodeRef as Parent;

impl VisitAstPath for ScopeCollector {
    fn visit_arrow_expr<'ast: 'r, 'r>(
        &mut self,
        node: &'ast ast::ArrowExpr,
        path: &mut AstNodePath<'r>,
    ) {
        let name = infer_name_from_ctx(path);

        self.scopes.push((convert_span(node.span), name));

        node.visit_children_with_path(self, path);
    }

    fn visit_function<'ast: 'r, 'r>(
        &mut self,
        node: &'ast ast::Function,
        path: &mut AstNodePath<'r>,
    ) {
        let ident = match path.last() {
            Some(Parent::FnDecl(fn_decl, _)) => Some(fn_decl.ident.clone()),
            Some(Parent::FnExpr(fn_expr, _)) => fn_expr.ident.clone(),
            _ => None,
        };
        let name = name_from_ident_or_ctx(ident, path);

        self.scopes.push((convert_span(node.span), name));

        node.visit_children_with_path(self, path);
    }

    // NOTE: instead of using `visit_constructor` here to find just a class constructor,
    // we want to find the whole class body, as class property definitions are executed as
    // part of the constructor.
    fn visit_class<'ast: 'r, 'r>(&mut self, node: &'ast ast::Class, path: &mut AstNodePath<'r>) {
        let ident = match path.last() {
            Some(Parent::ClassDecl(class_decl, _)) => Some(class_decl.ident.clone()),
            Some(Parent::ClassExpr(class_expr, _)) => class_expr.ident.clone(),
            _ => None,
        };
        let mut name = name_from_ident_or_ctx(ident, path);
        if let Some(name) = &mut name {
            name.components.push_front(NameComponent::interp("new "));
        }

        self.scopes.push((convert_span(node.span), name));

        node.visit_children_with_path(self, path);
    }
}

/// Uses either the provided [`ast::Ident`] or infers the name from the `path`.
fn name_from_ident_or_ctx(ident: Option<ast::Ident>, path: &AstNodePath) -> Option<ScopeName> {
    match ident {
        Some(ident) => {
            let mut name = ScopeName::new();
            name.components.push_back(NameComponent::ident(ident));
            Some(name)
        }
        None => infer_name_from_ctx(path),
    }
}

/// Tries to infer a name by walking up the path of ancestors.
fn infer_name_from_ctx(path: &AstNodePath) -> Option<ScopeName> {
    let mut scope_name = ScopeName::new();

    fn push_sep(name: &mut ScopeName) {
        if !name.components.is_empty() {
            name.components.push_front(NameComponent::interp("."));
        }
    }

    for parent in path.iter().rev() {
        match parent {
            // These create a new scope. If we reached this, it means we didn’t
            // use any of the other parents properly.
            Parent::Function(..) | Parent::ArrowExpr(..) | Parent::Constructor(..) => return None,

            // A class which is the parent of a method for which we already
            // have part of the name.
            Parent::ClassExpr(class_expr, _) => {
                if let Some(ident) = &class_expr.ident {
                    scope_name
                        .components
                        .push_front(NameComponent::ident(ident.clone()));
                }
            }
            Parent::ClassDecl(class_decl, _) => {
                push_sep(&mut scope_name);
                scope_name
                    .components
                    .push_front(NameComponent::ident(class_decl.ident.clone()));
                return Some(scope_name);
            }

            // An object literal member:
            // `{ $name() ... }`
            Parent::MethodProp(method, _) => {
                if let Some(ident) = method.key.as_ident() {
                    scope_name
                        .components
                        .push_front(NameComponent::ident(ident.clone()));
                }
            }

            // An object literal property:
            // `{ $name: ... }`
            Parent::KeyValueProp(kv, _) => {
                if let Some(ident) = kv.key.as_ident() {
                    scope_name
                        .components
                        .push_front(NameComponent::ident(ident.clone()));
                }
            }

            // A class method:
            // `class { $name() ... }`
            Parent::ClassMethod(method, _) => {
                if let Some(ident) = method.key.as_ident() {
                    scope_name
                        .components
                        .push_front(NameComponent::ident(ident.clone()));
                }
            }

            // A private class method:
            // `class { #$name() ... }`
            Parent::PrivateMethod(method, _) => {
                scope_name
                    .components
                    .push_front(NameComponent::ident(method.key.id.clone()));
                scope_name.components.push_front(NameComponent::interp("#"));
            }

            // A variable declaration with a name:
            // `var $name = ...`
            Parent::VarDeclarator(decl, _) => {
                if let Some(ident) = decl.name.as_ident() {
                    push_sep(&mut scope_name);
                    scope_name
                        .components
                        .push_front(NameComponent::ident(ident.id.clone()));
                    return Some(scope_name);
                }
            }

            // An assignment expression with a usable name on the left hand side
            // `$name = ...`
            Parent::AssignExpr(expr, _) => match &expr.left {
                ast::PatOrExpr::Expr(expr) => {
                    if let Some(mut expr_name) = infer_name_from_expr(expr) {
                        push_sep(&mut scope_name);

                        expr_name.components.append(&mut scope_name.components);
                        scope_name.components = expr_name.components;

                        return Some(scope_name);
                    }
                }
                ast::PatOrExpr::Pat(pat) => match pat.as_ref() {
                    ast::Pat::Ident(ident) => {
                        push_sep(&mut scope_name);
                        scope_name
                            .components
                            .push_front(NameComponent::ident(ident.id.clone()));
                        return Some(scope_name);
                    }
                    ast::Pat::Expr(expr) => {
                        if let Some(mut expr_name) = infer_name_from_expr(expr) {
                            push_sep(&mut scope_name);

                            expr_name.components.append(&mut scope_name.components);
                            scope_name.components = expr_name.components;

                            return Some(scope_name);
                        }
                    }
                    _ => {}
                },
            },

            _ => {}
        }
    }

    None
}

/// Returns a [`ScopeName`] corresponding to the given [`ast::Expr`].
///
/// This is only possible if the expression is an identifier or a member expression.
fn infer_name_from_expr(mut expr: &ast::Expr) -> Option<ScopeName> {
    let mut scope_name = ScopeName::new();
    loop {
        match expr {
            ast::Expr::Ident(ident) => {
                scope_name
                    .components
                    .push_front(NameComponent::ident(ident.clone()));
                return Some(scope_name);
            }

            ast::Expr::Member(member) => {
                if let Some(ident) = member.prop.as_ident() {
                    scope_name
                        .components
                        .push_front(NameComponent::ident(ident.clone()));
                    scope_name.components.push_front(NameComponent::interp("."));
                }
                expr = &member.obj;
            }

            ast::Expr::This(..) => {
                scope_name
                    .components
                    .push_front(NameComponent::interp("this"));
                return Some(scope_name);
            }

            _ => return None,
        }
    }
}
