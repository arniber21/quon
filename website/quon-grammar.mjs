// Quon TextMate grammar for Shiki syntax highlighting.
// Derived from tree-sitter-quon/grammar.js — provides keyword, type,
// comment, number, string, and operator highlighting for ```qn code blocks.
export const quonGrammar = {
  scopeName: 'source.quon',
  patterns: [
    { include: '#comments' },
    { include: '#keywords' },
    { include: '#types' },
    { include: '#constants' },
    { include: '#numbers' },
    { include: '#strings' },
    { include: '#operators' },
    { include: '#functions' },
    { include: '#identifiers' },
  ],
  repository: {
    comments: {
      patterns: [
        { name: 'comment.line.quon', match: '--[^\\n]*$' },
        { name: 'comment.block.quon', begin: '\\{-', end: '-\\}', patterns: [{ include: '#comments' }] },
      ],
    },
    keywords: {
      patterns: [
        { name: 'keyword.control.quon', match: '\\b(if|then|else|match|for|in|return|let|repeat)\\b' },
        { name: 'keyword.declaration.quon', match: '\\b(fn|type|circuit|run|borrow|par)\\b' },
        { name: 'keyword.other.quon', match: '\\b(adjoint|controlled|identity|measure|measure_all|qinit|qreg|destructure|split|tensored|on_high|swap_reverse|qubits|range|pairs|discard|reset)\\b' },
      ],
    },
    types: {
      patterns: [
        { name: 'support.type.quon', match: '\\b(Circuit|Qubit|QReg|QecBlock|Bit|Bool|Int|Float|Nat|List|Unit)\\b' },
        { name: 'support.class.quon', match: '\\b(Clifford|Universal|Repetition|Surface)\\b' },
        { name: 'support.constant.quon', match: '\\bQ\\b' },
      ],
    },
    constants: {
      patterns: [
        { name: 'constant.language.quon', match: '\\b(true|false)\\b' },
        { name: 'constant.numeric.quon', match: '\\bPI\\b' },
      ],
    },
    numbers: {
      patterns: [
        { name: 'constant.numeric.float.quon', match: '-?\\d+\\.\\d+([eE][+-]?\\d+)?' },
        { name: 'constant.numeric.integer.quon', match: '-?\\d+([eE][+-]?\\d+)?' },
      ],
    },
    strings: {
      patterns: [
        { name: 'string.quoted.double.quon', begin: '"', end: '"', patterns: [{ name: 'constant.character.escape.quon', match: '\\\\.' }] },
      ],
    },
    operators: {
      patterns: [
        { name: 'keyword.operator.compose.quon', match: '\\|>' },
        { name: 'keyword.operator.bind.quon', match: '<-' },
        { name: 'keyword.operator.arrow.quon', match: '->|-o|=>' },
        { name: 'keyword.operator.placement.quon', match: '@' },
        { name: 'keyword.operator.quon', match: '[+*/^|=]' },
        { name: 'keyword.operator.minus.quon', match: '-' },
        { name: 'punctuation.quon', match: '[:,.]' },
      ],
    },
    functions: {
      patterns: [
        { name: 'entity.name.function.quon', match: '\\b([A-Za-z_][A-Za-z0-9_]*)\\s*(?=\\()' },
      ],
    },
    identifiers: {
      patterns: [
        { name: 'variable.quon', match: '\\b[A-Za-z_][A-Za-z0-9_]*\\b' },
      ],
    },
  },
};
