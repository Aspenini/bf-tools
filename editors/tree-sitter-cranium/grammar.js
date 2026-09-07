/**
 * Tree-sitter grammar for Cranium.
 *
 * This mirrors `crates/cranium/src/parser.rs`. It is deliberately a little
 * looser than the compiler: an editor has to do something sensible with
 * half-written code, so this grammar is happy to parse things the compiler
 * would later reject. What it must get right is the shape - where names,
 * types, strings and comments are - because that is what highlighting and the
 * outline read.
 *
 * @file Cranium grammar for tree-sitter
 * @license MIT
 */

/* eslint-disable arrow-parens */
/* global grammar, seq, choice, repeat, repeat1, optional, prec, field, token, alias */

// Binding powers, loosest first, matching the compiler's expression chain.
const PRECEDENCE = {
  or: 1,
  and: 2,
  compare: 3,
  bitor: 4,
  bitand: 5,
  shift: 6,
  add: 7,
  multiply: 8,
  cast: 9,
  unary: 10,
  call: 11,
  index: 12,
};

module.exports = grammar({
  name: 'cranium',

  extras: ($) => [/\s/, $.line_comment, $.block_comment],

  word: ($) => $.identifier,

  rules: {
    source_file: ($) => repeat($._item),

    _item: ($) =>
      choice(
        $.import,
        $.const_item,
        $.global_item,
        $.function_item,
      ),

    import: ($) => seq('import', field('path', $.string), ';'),

    const_item: ($) =>
      seq(
        'const',
        field('name', $.identifier),
        optional(seq(':', field('type', $.type))),
        '=',
        field('value', $._expression),
        ';',
      ),

    global_item: ($) =>
      seq(
        'let',
        field('name', $.identifier),
        optional(seq(':', field('type', $.type))),
        optional(seq('=', field('value', $._expression))),
        ';',
      ),

    function_item: ($) =>
      seq(
        'fn',
        field('name', $.identifier),
        field('parameters', $.parameters),
        optional(seq('->', field('return_type', $.type))),
        field('body', $.block),
      ),

    parameters: ($) => seq('(', optional(commaSeparated($.parameter)), ')'),

    parameter: ($) =>
      seq(field('name', $.identifier), ':', field('type', $.type)),

    // Greedy, so `x as byte[4]` reads the whole type rather than stopping at
    // `byte` and treating `[4]` as an index. The compiler's parser does the
    // same thing.
    type: ($) => prec.right(seq($.primitive_type, repeat($.array_length))),

    array_length: ($) => seq('[', $.integer, ']'),

    primitive_type: () => choice('byte', 'int', 'sbyte', 'sint', 'bool'),

    // ---- statements ----------------------------------------------------

    block: ($) => seq('{', repeat($._statement), '}'),

    _statement: ($) =>
      choice(
        $.let_statement,
        $.assignment,
        $.if_statement,
        $.while_statement,
        $.loop_statement,
        $.for_statement,
        $.break_statement,
        $.continue_statement,
        $.return_statement,
        $.expression_statement,
        $.block,
      ),

    let_statement: ($) =>
      seq(
        'let',
        field('name', $.identifier),
        optional(seq(':', field('type', $.type))),
        optional(seq('=', field('value', $._expression))),
        ';',
      ),

    assignment: ($) =>
      seq(
        field('target', $._expression),
        field('operator', choice('=', '+=', '-=', '*=', '/=', '%=')),
        field('value', $._expression),
        ';',
      ),

    if_statement: ($) =>
      prec.right(
        seq(
          'if',
          field('condition', $._expression),
          field('consequence', $.block),
          optional(
            seq('else', field('alternative', choice($.block, $.if_statement))),
          ),
        ),
      ),

    while_statement: ($) =>
      seq('while', field('condition', $._expression), field('body', $.block)),

    loop_statement: ($) => seq('loop', field('body', $.block)),

    for_statement: ($) =>
      seq(
        'for',
        field('name', $.identifier),
        'in',
        field('start', $._expression),
        '..',
        field('end', $._expression),
        field('body', $.block),
      ),

    break_statement: () => seq('break', ';'),
    continue_statement: () => seq('continue', ';'),
    return_statement: ($) => seq('return', optional($._expression), ';'),
    expression_statement: ($) => seq($._expression, ';'),

    // ---- expressions ---------------------------------------------------

    _expression: ($) =>
      choice(
        $.identifier,
        $.integer,
        $.character,
        $.string,
        $.boolean,
        $.array_literal,
        $.call,
        $.index,
        $.unary_expression,
        $.binary_expression,
        $.cast_expression,
        $.parenthesized_expression,
      ),

    parenthesized_expression: ($) => seq('(', $._expression, ')'),

    array_literal: ($) => seq('[', optional(commaSeparated($._expression)), ']'),

    call: ($) =>
      prec(
        PRECEDENCE.call,
        seq(field('function', $.identifier), field('arguments', $.arguments)),
      ),

    arguments: ($) => seq('(', optional(commaSeparated($._expression)), ')'),

    index: ($) =>
      prec(
        PRECEDENCE.index,
        seq(field('array', $._expression), '[', field('index', $._expression), ']'),
      ),

    unary_expression: ($) =>
      prec(PRECEDENCE.unary, seq(choice('!', '-'), $._expression)),

    cast_expression: ($) =>
      prec.left(
        PRECEDENCE.cast,
        seq(field('value', $._expression), 'as', field('type', $.type)),
      ),

    binary_expression: ($) => {
      const table = [
        [PRECEDENCE.or, '||'],
        [PRECEDENCE.and, '&&'],
        [PRECEDENCE.compare, choice('==', '!=', '<', '<=', '>', '>=')],
        [PRECEDENCE.bitor, choice('|', '^')],
        [PRECEDENCE.bitand, '&'],
        [PRECEDENCE.shift, choice('<<', '>>')],
        [PRECEDENCE.add, choice('+', '-')],
        [PRECEDENCE.multiply, choice('*', '/', '%')],
      ];
      return choice(
        ...table.map(([precedence, operator]) =>
          prec.left(
            precedence,
            seq(
              field('left', $._expression),
              field('operator', operator),
              field('right', $._expression),
            ),
          ),
        ),
      );
    },

    // ---- tokens ---------------------------------------------------------

    identifier: () => /[A-Za-z_][A-Za-z0-9_]*/,

    boolean: () => choice('true', 'false'),

    integer: () =>
      token(choice(/0[xX][0-9a-fA-F_]+/, /0[bB][01_]+/, /[0-9][0-9_]*/)),

    character: ($) => seq("'", choice($.escape_sequence, /[^'\\\n]/), "'"),

    string: ($) => seq('"', repeat(choice($.escape_sequence, /[^"\\\n]+/)), '"'),

    escape_sequence: () => token.immediate(/\\(x[0-9a-fA-F]{2}|[nrt0e\\'"])/),

    line_comment: () => token(seq('//', /[^\n]*/)),

    block_comment: () => token(seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/')),
  },
});

/** A comma-separated list with no trailing comma. */
function commaSeparated(rule) {
  return seq(rule, repeat(seq(',', rule)));
}
