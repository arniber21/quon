"use strict";

/**
 * Node entry for tree-sitter-quon.
 *
 * This package ships grammar sources (`grammar.js`, `src/parser.c`, `queries/`),
 * not a prebuilt native addon under `bindings/node`. Editor consumers (#132/#133)
 * should point Tree-sitter tooling at this directory; `require("tree-sitter-quon")`
 * only exposes path metadata so the package resolves without MODULE_NOT_FOUND.
 */
const path = require("path");

module.exports = {
  name: "tree-sitter-quon",
  grammarJs: path.join(__dirname, "grammar.js"),
  parserC: path.join(__dirname, "src", "parser.c"),
  nodeTypes: path.join(__dirname, "src", "node-types.json"),
  queriesDir: path.join(__dirname, "queries"),
  highlightsQuery: path.join(__dirname, "queries", "highlights.scm"),
};
