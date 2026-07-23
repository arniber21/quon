// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// Quon TextMate grammar for Shiki syntax highlighting.
// Derived from tree-sitter-quon/grammar.js — provides keyword, type,
// comment, number, string, and operator highlighting for ```qn code blocks.
const quonGrammar = {
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

// https://astro.build/config
export default defineConfig({
	site: 'https://quon.arnabg.me',
	markdown: {
		shikiConfig: {
			langs: [
				{ id: 'qn', scopeName: 'source.quon', grammar: quonGrammar },
			],
			langAlias: {
				mlir: 'llvm',
				qasm: 'verilog',
			},
		},
	},
	integrations: [
		starlight({
			title: 'Quon',
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/arniber21/quon' },
			],
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						{ label: 'Install Quon', slug: 'getting-started/install' },
						{ label: 'Quickstart', slug: 'getting-started/quickstart' },
						{ label: 'Your second program', slug: 'getting-started/second-program' },
					],
				},
				{
					label: 'Why Quon',
					items: [
						{ label: 'Design philosophy', slug: 'why-quon/philosophy' },
					],
				},
				{
					label: 'Language guide',
					items: [
						{ label: 'Introduction', slug: 'language/introduction' },
						{ label: 'Circuits and gates', slug: 'language/circuits' },
						{ label: 'Qubits and registers', slug: 'language/qubits' },
						{ label: 'The linear type system', slug: 'language/linearity' },
						{ label: 'Parallel composition', slug: 'language/parallel' },
						{ label: 'Depth bounds', slug: 'language/depth' },
						{ label: 'Clifford classification', slug: 'language/clifford' },
						{ label: 'The Quantum Monad', slug: 'language/monad' },
						{ label: 'Measurement and control', slug: 'language/measurement' },
						{ label: 'Borrow blocks', slug: 'language/borrow' },
						{ label: 'QEC blocks', slug: 'language/qec' },
						{ label: 'Putting it together', slug: 'language/putting-together' },
					],
				},
				{
					label: 'Cookbook',
					items: [
						{ label: 'Overview', slug: 'cookbook/index' },
						{ label: 'Bell state', slug: 'cookbook/bell' },
						{ label: 'Teleportation', slug: 'cookbook/teleportation' },
						{ label: 'Bernstein–Vazirani', slug: 'cookbook/bernstein-vazirani' },
						{ label: 'Grover search', slug: 'cookbook/grover' },
						{ label: 'Quantum Fourier transform', slug: 'cookbook/qft' },
						{ label: 'Transverse-field Ising', slug: 'cookbook/ising' },
						{ label: 'QAOA MaxCut', slug: 'cookbook/qaoa' },
						{ label: 'Shor quantum kernel', slug: 'cookbook/shor-kernel' },
						{ label: 'More samples', slug: 'cookbook/samples' },
						{ label: 'NA QAOA schedule', slug: 'cookbook/na-qaoa' },
					],
				},
				{
					label: 'Architecture',
					items: [
						{ label: 'Compiler internals', slug: 'architecture/compiler-internals' },
						{ label: 'Neutral-atom model', slug: 'architecture/na-model' },
					],
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Developer tooling', slug: 'guides/tooling' },
						{ label: 'Backends and verification', slug: 'guides/backends' },
						{ label: 'Neutral-atom FT demo', slug: 'guides/na-ft-demo' },
						{ label: 'Maturation path', slug: 'guides/roadmap' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'quonc CLI', slug: 'reference/quonc' },
						{ label: 'Compiler pipeline', slug: 'reference/compiler' },
					],
				},
			],
		}),
	],
});
