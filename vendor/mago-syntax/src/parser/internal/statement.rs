use mago_allocator::prelude::*;
use mago_database::file::HasFileId;
use mago_span::Span;

use crate::T;
use crate::cst::cst::AttributeList;
use crate::cst::cst::Expression;
use crate::cst::cst::ExpressionStatement;
use crate::cst::cst::Statement;
use crate::cst::sequence::Sequence;
use crate::error::ParseError;
use crate::parser::MAX_RECURSION_DEPTH;
use crate::parser::Parser;

impl<'arena, A> Parser<'_, 'arena, A>
where
    A: Arena,
{
    pub(crate) fn parse_statement(&mut self) -> Result<Statement<'arena>, ParseError> {
        self.state.recursion_depth += 1;
        if self.state.recursion_depth > MAX_RECURSION_DEPTH {
            self.state.recursion_depth -= 1;
            let file_id = self.stream.file_id();

            return Err(ParseError::RecursionLimitExceeded(
                self.stream.lookahead(0)?.map(|t| t.span_for(file_id)).unwrap_or_else(|| {
                    Span::new(file_id, self.stream.current_position(), self.stream.current_position())
                }),
            ));
        }

        let result = self.parse_statement_inner();
        self.state.recursion_depth -= 1;
        result
    }

    fn parse_statement_inner(&mut self) -> Result<Statement<'arena>, ParseError> {
        let token = self.stream.lookahead(0)?.ok_or_else(|| self.stream.unexpected(None, &[]))?;
        Ok(match token.kind {
            T![InlineText | InlineShebang] => Statement::Inline(self.arena.alloc(self.parse_inline()?)),
            T!["<?php"] | T!["<?"] => Statement::OpeningTag(self.arena.alloc(self.parse_opening_tag()?)),
            T!["<?="] => Statement::EchoTag(self.arena.alloc(self.parse_echo_tag()?)),
            T!["?>"] => Statement::ClosingTag(self.arena.alloc(self.parse_closing_tag()?)),
            T!["declare"] => Statement::Declare(self.arena.alloc(self.parse_declare()?)),
            T!["namespace"] => Statement::Namespace(self.arena.alloc(self.parse_namespace()?)),
            T!["use"] => Statement::Use(self.arena.alloc(self.parse_use()?)),
            T!["return"] => Statement::Return(self.arena.alloc(self.parse_return()?)),
            T!["#["] => {
                let attributes = self.parse_attribute_list_sequence()?;
                let next = self.stream.lookahead(0)?.ok_or_else(|| self.stream.unexpected(None, &[]))?;
                let maybe_after = self.stream.peek_kind(1)?;

                match next.kind {
                    T!["interface"] => Statement::Interface(self.arena.alloc(self.parse_interface_with_attributes(attributes)?)),
                    T!["trait"] => Statement::Trait(self.arena.alloc(self.parse_trait_with_attributes(attributes)?)),
                    T!["enum"] => Statement::Enum(self.arena.alloc(self.parse_enum_with_attributes(attributes)?)),
                    T!["class"] => Statement::Class(self.arena.alloc(self.parse_class_with_attributes(attributes)?)),
                    T!["const"] => Statement::Constant(self.arena.alloc(self.parse_constant_with_attributes(attributes)?)),
                    T!["function"] => self.parse_closure_or_function(attributes)?,
                    T!["fn"] => Statement::Expression(self.arena.alloc(ExpressionStatement {
                        expression: {
                            self.arena.alloc(Expression::ArrowFunction(
                                self.arena.alloc(self.parse_arrow_function_with_attributes(attributes)?),
                            ))
                        },
                        terminator: self.parse_terminator()?,
                    })),
                    T!["static"] if maybe_after == Some(T!["fn"]) => Statement::Expression(self.arena.alloc(ExpressionStatement {
                        expression: {
                            self.arena.alloc(Expression::ArrowFunction(
                                self.arena.alloc(self.parse_arrow_function_with_attributes(attributes)?),
                            ))
                        },
                        terminator: self.parse_terminator()?,
                    })),
                    T!["static"] if maybe_after == Some(T!["function"]) => Statement::Expression(self.arena.alloc(ExpressionStatement {
                        expression: {
                            self.arena.alloc(Expression::Closure(self.arena.alloc(self.parse_closure_with_attributes(attributes)?)))
                        },
                        terminator: self.parse_terminator()?,
                    })),
                    kind if kind.is_modifier() => Statement::Class(self.arena.alloc(self.parse_class_with_attributes(attributes)?)),
                    _ => {
                        return Err(self.stream.unexpected(
                            Some(next),
                            T![
                                "interface",
                                "trait",
                                "enum",
                                "class",
                                "function",
                                "fn",
                                "readonly",
                                "abstract",
                                "final",
                                "static",
                            ],
                        ));
                    }
                }
            }
            T!["interface"] => Statement::Interface(self.arena.alloc(self.parse_interface_with_attributes(Sequence::empty())?)),
            T!["trait"] => Statement::Trait(self.arena.alloc(self.parse_trait_with_attributes(Sequence::empty())?)),
            T!["enum"] => Statement::Enum(self.arena.alloc(self.parse_enum_with_attributes(Sequence::empty())?)),
            T!["class"] => Statement::Class(self.arena.alloc(self.parse_class_with_attributes(Sequence::empty())?)),
            T!["function"] => self.parse_closure_or_function(Sequence::empty())?,
            T!["global"] => Statement::Global(self.arena.alloc(self.parse_global()?)),
            T!["static"] if matches!(self.stream.peek_kind(1)?, Some(T!["$variable"])) => {
                Statement::Static(self.arena.alloc(self.parse_static()?))
            }
            kind if kind.is_modifier()
                && !matches!(
                    self.stream.peek_kind(1)?,
                    Some(T!["::" | "(" | "->" | "?->" | "[" | "fn" | "function"])
                ) =>
            {
                Statement::Class(self.arena.alloc(self.parse_class_with_attributes(Sequence::empty())?))
            }
            T!["__halt_compiler"] => Statement::HaltCompiler(self.arena.alloc(self.parse_halt_compiler()?)),
            T![";"] => Statement::Noop(self.arena.alloc(self.stream.consume_span()?)),
            T!["const"] => Statement::Constant(self.arena.alloc(self.parse_constant_with_attributes(Sequence::empty())?)),
            T!["if"] => Statement::If(self.arena.alloc(self.parse_if()?)),
            T!["switch"] => Statement::Switch(self.arena.alloc(self.parse_switch()?)),
            T!["foreach"] => Statement::Foreach(self.arena.alloc(self.parse_foreach()?)),
            T!["for"] => Statement::For(self.arena.alloc(self.parse_for()?)),
            T!["while"] => Statement::While(self.arena.alloc(self.parse_while()?)),
            T!["do"] => Statement::DoWhile(self.arena.alloc(self.parse_do_while()?)),
            T!["continue"] => Statement::Continue(self.arena.alloc(self.parse_continue()?)),
            T!["break"] => Statement::Break(self.arena.alloc(self.parse_break()?)),
            T!["unset"] => Statement::Unset(self.arena.alloc(self.parse_unset()?)),
            T!["{"] => Statement::Block(self.arena.alloc(self.parse_block()?)),
            T!["try"] => Statement::Try(self.arena.alloc(self.parse_try()?)),
            T!["echo"] => Statement::Echo(self.arena.alloc(self.parse_echo()?)),
            T!["goto"] => Statement::Goto(self.arena.alloc(self.parse_goto()?)),
            kind if kind.is_identifier_maybe_reserved() && matches!(self.stream.peek_kind(1)?, Some(T![":"])) => {
                Statement::Label(self.arena.alloc(self.parse_label()?))
            }
            _ => Statement::Expression(self.arena.alloc(ExpressionStatement {
                expression: self.parse_expression()?,
                terminator: self.parse_terminator()?,
            })),
        })
    }

    fn parse_closure_or_function(
        &mut self,
        attributes: Sequence<'arena, AttributeList<'arena>>,
    ) -> Result<Statement<'arena>, ParseError> {
        Ok(match (self.stream.peek_kind(1)?, self.stream.lookahead(2)?.map(|t| t.kind)) {
            // if the next token is `(` or `&` followed by `(`, then we know this is a closure
            (Some(T!["("]), _) | (Some(T!["&"]), Some(T!["("])) => Statement::Expression(self.arena.alloc(ExpressionStatement {
                expression: { self.arena.alloc(Expression::Closure(self.arena.alloc(self.parse_closure_with_attributes(attributes)?))) },
                terminator: self.parse_terminator()?,
            })),
            _ => {
                // otherwise, we know this is a function
                Statement::Function(self.arena.alloc(self.parse_function_with_attributes(attributes)?))
            }
        })
    }
}
