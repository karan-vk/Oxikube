//! Reads a generated query back with a real PromQL parser.

use promql_parser::label::MatchOp;
use promql_parser::parser::{self, Expr};

/// One `label op value` of a selector in the query, as the parser read it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Parsed {
    pub metric: String,
    pub label: String,
    pub op: &'static str,
    pub value: String,
}

/// Parses `query`, or says why it is not PromQL.
pub fn parse(query: &str) -> Result<Expr, String> {
    parser::parse(query)
}

/// Every matcher of every selector in `query`.
pub fn matchers(query: &str) -> Vec<Parsed> {
    let expr = parse(query).unwrap_or_else(|e| panic!("{e}\n{query}"));
    let mut out = Vec::new();
    walk(&expr, &mut out);
    out
}

fn walk(expr: &Expr, out: &mut Vec<Parsed>) {
    match expr {
        Expr::Aggregate(a) => {
            walk(&a.expr, out);
            if let Some(param) = &a.param {
                walk(param, out);
            }
        }
        Expr::Unary(u) => walk(&u.expr, out),
        Expr::Binary(b) => {
            walk(&b.lhs, out);
            walk(&b.rhs, out);
        }
        Expr::Paren(p) => walk(&p.expr, out),
        Expr::Subquery(s) => walk(&s.expr, out),
        Expr::VectorSelector(v) => selector(v, out),
        Expr::MatrixSelector(m) => selector(&m.vs, out),
        Expr::Call(c) => c.args.args.iter().for_each(|a| walk(a, out)),
        Expr::NumberLiteral(_) | Expr::StringLiteral(_) | Expr::Extension(_) => {}
    }
}

fn selector(v: &promql_parser::parser::VectorSelector, out: &mut Vec<Parsed>) {
    for m in &v.matchers.matchers {
        let (op, value) = match &m.op {
            MatchOp::Equal => ("=", m.value.clone()),
            MatchOp::NotEqual => ("!=", m.value.clone()),
            MatchOp::Re(_) => ("=~", m.value.clone()),
            MatchOp::NotRe(_) => ("!~", m.value.clone()),
        };
        out.push(Parsed {
            metric: v.name.clone().unwrap_or_default(),
            label: m.name.clone(),
            op,
            value,
        });
    }
}
