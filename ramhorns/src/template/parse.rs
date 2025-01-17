// Ramhorns  Copyright (C) 2019  Maciej Hirsz
//
// This file is part of Ramhorns. This program comes with ABSOLUTELY NO WARRANTY;
// This is free software, and you are welcome to redistribute it under the
// conditions of the GNU General Public License version 3.0.
//
// You should have received a copy of the GNU General Public License
// along with Ramhorns.  If not, see <http://www.gnu.org/licenses/>

use arrayvec::ArrayVec;
use logos::Logos;

use super::{hash_name, Block, Error, Tag, Template};
use crate::Partials;

#[derive(Logos)]
#[logos(extras = Braces)]
enum Opening {
    #[token("{{", |_| Tag::Escaped)]
    #[token("{{&", |_| Tag::Unescaped)]
    #[token("{{{", |lex| {
        // Flag that we will expect 3 closing braces
        lex.extras = Braces::Three;

        Tag::Unescaped
    })]
    #[token("{{#", |_| Tag::Section)]
    #[token("{{^", |_| Tag::Inverse)]
    #[token("{{/", |_| Tag::Closing)]
    #[token("{{>", |_| Tag::Partial)]
    #[token("{{!", |_| Tag::Comment)]
    Match(Tag),

    #[regex(r"[^{]+", logos::skip)]
    #[token("{", logos::skip)]
    #[error]
    Err,
}

#[derive(Logos)]
#[logos(extras = Braces)]
enum Closing {
    #[token("}}", |lex| {
        // Force fail the match if we expected 3 braces
        lex.extras != Braces::Three
    })]
    #[token("}}}")]
    Match,

    #[regex(r"[^ \}]+")]
    Ident,

    #[regex(r"[ ]+", logos::skip)]
    #[error]
    Err,
}

/// Marker of how many braces we expect to match
#[derive(PartialEq, Eq, Clone, Copy)]
enum Braces {
    Two = 2,
    Three = 3,
}

impl Default for Braces {
    #[inline]
    fn default() -> Self {
        Braces::Two
    }
}

impl<'tpl> Template<'tpl> {
    pub(crate) fn parse(
        &mut self,
        source: &'tpl str,
        partials: &mut impl Partials<'tpl>,
    ) -> Result<usize, Error> {
        let mut last = 0;
        let mut lex = Opening::lexer(source);
        let mut stack = ArrayVec::<usize, 16>::new();

        while let Some(token) = lex.next() {
            let tag = match token {
                Opening::Match(tag) => tag,
                Opening::Err => return Err(Error::UnclosedTag),
            };

            // Grab HTML from before the token
            // TODO: add lex.before() that yields source slice
            // in front of the token:
            //
            // let html = &lex.before()[last..];
            let mut html = &lex.source()[last..lex.span().start];
            self.capacity_hint += html.len();

            // Morphing the lexer to match the closing
            // braces and grab the name
            let mut closing = lex.morph();
            let tail_idx = self.blocks.len();

            let _tok = closing.next();
            if !matches!(Some(Closing::Ident), _tok) {
                return Err(Error::UnclosedTag);
            }
            let mut name = closing.slice();
                    
            match tag {
                Tag::Escaped | Tag::Unescaped => {
                    loop {
                        match closing.next() {
                            Some(Closing::Ident) => {
                                self.blocks.push(Block::new(html, name, Tag::Section));
                                name = closing.slice();
                                html = "";
                            },
                            Some(Closing::Match) => {
                                self.blocks.push(Block::new(html, name, tag));
                                break;
                            }
                            _ => return Err(Error::UnclosedTag),
                        }
                    }
                    
                    let d = self.blocks.len() - tail_idx - 1;
                    for i in 0..d {
                        self.blocks[tail_idx + i].children = (d - i) as u32;
                    }
                }
                Tag::Section | Tag::Inverse => {
                    loop {
                        match closing.next() {
                            Some(Closing::Ident) => {
                                stack.try_push(self.blocks.len())?;
                                self.blocks.push(Block::new(html, name, Tag::Section));
                                name = closing.slice();
                                html = "";
                            },
                            Some(Closing::Match) => {
                                stack.try_push(self.blocks.len())?;
                                self.blocks.push(Block::new(html, name, tag));
                                break;
                            }
                            _ => return Err(Error::UnclosedTag),
                        }
                    }
                }
                Tag::Closing => {
                    self.blocks.push(Block::nameless(html, Tag::Closing));

                    let mut pop_section = |name| {
                        let hash = hash_name(name);

                        let head_idx = stack
                            .pop()
                            .ok_or_else(|| Error::UnopenedSection(name.into()))?;
                        let head = &mut self.blocks[head_idx];
                        head.children = (tail_idx - head_idx) as u32;

                        if head.hash != hash {
                            return Err(Error::UnclosedSection(head.name.into()));
                        }
                        Ok(())
                    };
                    
                    pop_section(name)?;
                    loop {
                        match closing.next() {
                            Some(Closing::Ident) => {
                                pop_section(closing.slice())?;
                            },
                            Some(Closing::Match) => break,
                            _ => return Err(Error::UnclosedTag),
                        }
                    }
                }
                Tag::Partial => {
                    match closing.next() {
                        Some(Closing::Match) => {},
                        _ => return Err(Error::UnclosedTag),
                    }
                    
                    self.blocks.push(Block::nameless(html, tag));
                    let partial = partials.get_partial(name)?;
                    self.blocks.extend_from_slice(&partial.blocks);
                    self.capacity_hint += partial.capacity_hint;
                }
                _ => {
                    loop {
                        match closing.next() {
                            Some(Closing::Ident) => continue,
                            Some(Closing::Match) => break,
                            _ => return Err(Error::UnclosedTag),
                        }
                    }
                    self.blocks.push(Block::nameless(html, tag));
                }
            };

            // Add the number of braces that we were expecting,
            // not the number we got:
            //
            // `{{foo}}}` should not consume the last `}`
            last = closing.span().start + closing.extras as usize;
            lex = closing.morph();
            lex.extras = Braces::Two;
        }

        Ok(last)
    }
}
