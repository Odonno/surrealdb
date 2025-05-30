use std::collections::BTreeMap;

use reblessive::Stk;

use crate::{
	expr::{Array, Duration, Ident, Object, Strand, Value},
	syn::{
		lexer::compound::{self, Numeric},
		parser::mac::{expected, pop_glued},
		token::{Glued, Span, TokenKind, t},
	},
};

use super::{ParseResult, Parser};

impl Parser<'_> {
	pub async fn parse_json(&mut self, ctx: &mut Stk) -> ParseResult<Value> {
		let token = self.peek();
		match token.kind {
			t!("NULL") => {
				self.pop_peek();
				Ok(Value::Null)
			}
			t!("true") => {
				self.pop_peek();
				Ok(Value::Bool(true))
			}
			t!("false") => {
				self.pop_peek();
				Ok(Value::Bool(false))
			}
			t!("{") => {
				self.pop_peek();
				self.parse_json_object(ctx, token.span).await.map(Value::Object)
			}
			t!("[") => {
				self.pop_peek();
				self.parse_json_array(ctx, token.span).await.map(Value::Array)
			}
			t!("\"") | t!("'") => {
				let strand: Strand = self.next_token_value()?;
				if self.settings.legacy_strands {
					if let Some(x) = self.reparse_legacy_strand(ctx, &strand.0).await {
						return Ok(x);
					}
				}
				Ok(Value::Strand(strand))
			}
			t!("-") | t!("+") | TokenKind::Digits => {
				self.pop_peek();
				let compound = self.lexer.lex_compound(token, compound::numeric)?;
				match compound.value {
					Numeric::Duration(x) => Ok(Value::Duration(Duration(x))),
					Numeric::Number(x) => Ok(Value::Number(x)),
				}
			}
			TokenKind::Glued(Glued::Strand) => {
				let glued = pop_glued!(self, Strand);
				Ok(Value::Strand(glued))
			}
			TokenKind::Glued(Glued::Duration) => {
				let glued = pop_glued!(self, Duration);
				Ok(Value::Duration(glued))
			}
			_ => {
				let ident = self.next_token_value::<Ident>()?.0;
				self.parse_thing_from_ident(ctx, ident).await.map(Value::Thing)
			}
		}
	}

	async fn parse_json_object(&mut self, ctx: &mut Stk, start: Span) -> ParseResult<Object> {
		let mut obj = BTreeMap::new();
		loop {
			if self.eat(t!("}")) {
				return Ok(Object(obj));
			}
			let key = self.parse_object_key()?;
			expected!(self, t!(":"));
			let value = ctx.run(|ctx| self.parse_json(ctx)).await?;
			obj.insert(key, value);

			if !self.eat(t!(",")) {
				self.expect_closing_delimiter(t!("}"), start)?;
				return Ok(Object(obj));
			}
		}
	}

	async fn parse_json_array(&mut self, ctx: &mut Stk, start: Span) -> ParseResult<Array> {
		let mut array = Vec::new();
		loop {
			if self.eat(t!("]")) {
				return Ok(Array(array));
			}
			let value = ctx.run(|ctx| self.parse_json(ctx)).await?;
			array.push(value);

			if !self.eat(t!(",")) {
				self.expect_closing_delimiter(t!("]"), start)?;
				return Ok(Array(array));
			}
		}
	}
}
