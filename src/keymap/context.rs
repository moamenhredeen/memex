#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyContext {
    entries: Vec<ContextEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContextEntry {
    key: &'static str,
    value: Option<&'static str>,
}

impl KeyContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, key: &'static str) {
        self.entries.push(ContextEntry { key, value: None });
    }

    pub fn set(&mut self, key: &'static str, value: &'static str) {
        self.entries.push(ContextEntry {
            key,
            value: Some(value),
        });
    }

    pub fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|entry| entry.key == key)
    }

    pub fn matches_value(&self, key: &str, value: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.key == key && entry.value == Some(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPredicate {
    expr: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
    Always,
    Flag(&'static str),
    Equal(&'static str, &'static str),
    NotEqual(&'static str, &'static str),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

impl Default for KeyPredicate {
    fn default() -> Self {
        Self { expr: Expr::Always }
    }
}

impl KeyPredicate {
    pub fn always() -> Self {
        Self::default()
    }

    pub fn parse(source: &str) -> Result<Self, String> {
        let mut parser = Parser::new(source);
        let expr = parser.parse_or()?;
        if parser.peek().is_some() {
            return Err(format!("unexpected token in context: {:?}", parser.peek()));
        }
        Ok(Self { expr })
    }

    pub fn require(self, key: &'static str) -> Self {
        self.and(Self {
            expr: Expr::Flag(key),
        })
    }

    pub fn require_value(self, key: &'static str, value: &'static str) -> Self {
        self.and(Self {
            expr: Expr::Equal(key, value),
        })
    }

    pub fn forbid(self, key: &'static str) -> Self {
        self.and(Self {
            expr: Expr::Not(Box::new(Expr::Flag(key))),
        })
    }

    fn and(self, other: Self) -> Self {
        match (self.expr, other.expr) {
            (Expr::Always, expr) | (expr, Expr::Always) => Self { expr },
            (left, right) => Self {
                expr: Expr::And(Box::new(left), Box::new(right)),
            },
        }
    }

    pub fn matches(&self, context: &KeyContext) -> bool {
        self.expr.matches(context)
    }

    pub fn specificity(&self) -> usize {
        self.expr.specificity()
    }
}

impl Expr {
    fn matches(&self, context: &KeyContext) -> bool {
        match self {
            Expr::Always => true,
            Expr::Flag(key) => context.contains(key),
            Expr::Equal(key, value) => context.matches_value(key, value),
            Expr::NotEqual(key, value) => !context.matches_value(key, value),
            Expr::Not(expr) => !expr.matches(context),
            Expr::And(left, right) => left.matches(context) && right.matches(context),
            Expr::Or(left, right) => left.matches(context) || right.matches(context),
        }
    }

    fn specificity(&self) -> usize {
        match self {
            Expr::Always => 0,
            Expr::Flag(_) | Expr::Equal(_, _) | Expr::NotEqual(_, _) => 1,
            Expr::Not(expr) => expr.specificity(),
            Expr::And(left, right) => left.specificity() + right.specificity(),
            Expr::Or(left, right) => left.specificity().max(right.specificity()),
        }
    }
}

pub fn when() -> KeyPredicate {
    KeyPredicate::always()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    And,
    Or,
    Not,
    Eq,
    Ne,
    LParen,
    RParen,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(source: &str) -> Self {
        Self {
            tokens: tokenize(source),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.bump();
            let right = self.parse_and()?;
            expr = Expr::Or(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_unary()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.bump();
            let right = self.parse_unary()?;
            expr = Expr::And(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.bump();
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.bump() {
            Some(Token::Ident(key)) => {
                let key = leak(key);
                match self.peek() {
                    Some(Token::Eq) => {
                        self.bump();
                        let Some(Token::Ident(value)) = self.bump() else {
                            return Err("expected value after ==".into());
                        };
                        Ok(Expr::Equal(key, leak(value)))
                    }
                    Some(Token::Ne) => {
                        self.bump();
                        let Some(Token::Ident(value)) = self.bump() else {
                            return Err("expected value after !=".into());
                        };
                        Ok(Expr::NotEqual(key, leak(value)))
                    }
                    _ => Ok(Expr::Flag(key)),
                }
            }
            Some(Token::LParen) => {
                let expr = self.parse_or()?;
                match self.bump() {
                    Some(Token::RParen) => Ok(expr),
                    other => Err(format!("expected ')', got {other:?}")),
                }
            }
            other => Err(format!("expected context expression, got {other:?}")),
        }
    }
}

fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            c if c.is_whitespace() => i += 1,
            '&' if chars.get(i + 1) == Some(&'&') => {
                tokens.push(Token::And);
                i += 2;
            }
            '|' if chars.get(i + 1) == Some(&'|') => {
                tokens.push(Token::Or);
                i += 2;
            }
            '=' if chars.get(i + 1) == Some(&'=') => {
                tokens.push(Token::Eq);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'=') => {
                tokens.push(Token::Ne);
                i += 2;
            }
            '!' => {
                tokens.push(Token::Not);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            _ => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '-' | ':'))
                {
                    i += 1;
                }
                if i == start {
                    i += 1;
                } else {
                    tokens.push(Token::Ident(chars[start..i].iter().collect()));
                }
            }
        }
    }
    tokens
}

fn leak(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}
