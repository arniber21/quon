// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
	site: 'https://quon.arnabg.me',
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
				label: 'Learning track',
				items: [
					{ label: 'Overview', slug: 'learn' },
					{ label: '1 · Hello, Quon', slug: 'learn/01-hello-quon' },
					{ label: '2 · States & measurement', slug: 'learn/02-states-measurement' },
					{ label: '3 · Gates & composition', slug: 'learn/03-gates-composition' },
					{ label: '4 · Linearity & ancilla', slug: 'learn/04-linearity-borrow' },
					{ label: '5 · Entanglement', slug: 'learn/05-entanglement' },
					{ label: '6 · Oracles & algorithms', slug: 'learn/06-oracles-algorithms' },
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
						{ autogenerate: { directory: 'cookbook' } },
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
