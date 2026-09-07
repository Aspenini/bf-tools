; Ordering matters: tree-sitter resolves a node to the earliest pattern that
; matches it, so the specific cases come first and bare identifiers last.

; Builtins the compiler provides, rather than anything the program defined.
((call function: (identifier) @function.builtin)
 (#match? @function.builtin "^(print|println|putc|getc|puts|len)$"))

(function_item name: (identifier) @function.definition)
(call function: (identifier) @function)

(parameter name: (identifier) @variable.parameter)
(const_item name: (identifier) @constant)

(primitive_type) @type.builtin
(boolean) @constant.builtin

[
  "fn"
  "let"
  "const"
  "import"
  "return"
] @keyword

[
  "if"
  "else"
  "while"
  "for"
  "in"
  "loop"
  "break"
  "continue"
] @keyword.control

"as" @keyword.operator

[
  "+" "-" "*" "/" "%"
  "==" "!=" "<" "<=" ">" ">="
  "&&" "||" "!"
  "&" "|" "^" "<<" ">>"
  "=" "+=" "-=" "*=" "/=" "%="
  "->" ".."
] @operator

[ "(" ")" "[" "]" "{" "}" ] @punctuation.bracket
[ ";" "," ":" ] @punctuation.delimiter

(integer) @number
(character) @string.special.symbol
(string) @string
(escape_sequence) @string.escape

[ (line_comment) (block_comment) ] @comment

(identifier) @variable
