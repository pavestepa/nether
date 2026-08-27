use nether_ast::{BinaryOp, ExprKind};
use nether_lexer::{Punct, Token};

pub(super) fn is_block_like(kind: &ExprKind) -> bool {
    matches!(
        kind,
        ExprKind::If { .. }
            | ExprKind::Match { .. }
            | ExprKind::While { .. }
            | ExprKind::ForIn { .. }
            | ExprKind::Loop { .. }
            | ExprKind::Block(_)
            | ExprKind::Unsafe(_)
    )
}

pub(super) fn peek_binop(token: &Token) -> Option<(BinaryOp, u8, u8)> {
    let op = match token {
        Token::Punct(Punct::PipePipe) => BinaryOp::Or,
        Token::Punct(Punct::AmpAmp) => BinaryOp::And,
        Token::Punct(Punct::EqEq) => BinaryOp::Eq,
        Token::Punct(Punct::Ne) => BinaryOp::Ne,
        Token::Punct(Punct::Lt) => BinaryOp::Lt,
        Token::Punct(Punct::Le) => BinaryOp::Le,
        Token::Punct(Punct::Gt) => BinaryOp::Gt,
        Token::Punct(Punct::Ge) => BinaryOp::Ge,
        Token::Punct(Punct::Plus) => BinaryOp::Add,
        Token::Punct(Punct::Minus) => BinaryOp::Sub,
        Token::Punct(Punct::Star) => BinaryOp::Mul,
        Token::Punct(Punct::Slash) => BinaryOp::Div,
        Token::Punct(Punct::Percent) => BinaryOp::Rem,
        _ => return None,
    };
    let bp = match op {
        BinaryOp::Or => 1,
        BinaryOp::And => 2,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            3
        }
        BinaryOp::Add | BinaryOp::Sub => 4,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => 5,
    };
    Some((op, bp, bp + 1))
}
