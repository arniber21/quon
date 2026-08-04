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
					{ label: 'Choose your path', slug: 'getting-started/choose-your-path' },
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
						{ label: 'Cookbook', slug: 'cookbook' },
						{ label: 'Bell state', slug: 'cookbook/bell' },
						{ label: 'Quantum teleportation', slug: 'cookbook/teleportation' },
						{ label: 'Bernstein–Vazirani', slug: 'cookbook/bernstein-vazirani' },
						{ label: 'Grover search', slug: 'cookbook/grover' },
						{ label: 'Quantum Fourier transform', slug: 'cookbook/qft' },
						{ label: 'Transverse-field Ising', slug: 'cookbook/ising' },
						{ label: 'QAOA MaxCut', slug: 'cookbook/qaoa' },
						{ label: 'Shor quantum kernel', slug: 'cookbook/shor-kernel' },
						{ label: 'NA QAOA schedule', slug: 'cookbook/na-qaoa' },
						{
							label: 'Sample discovery (optional)',
							collapsed: true,
							items: [
								{ label: 'More samples', slug: 'cookbook/samples' },
								{ label: 'Deutsch–Jozsa', slug: 'cookbook/deutsch-jozsa' },
								{ label: "Simon's algorithm", slug: 'cookbook/simon' },
								{ label: 'Phase estimation', slug: 'cookbook/phase-estimation' },
							],
						},
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
						{ label: 'Results and plotting', slug: 'guides/results-and-plotting' },
						{ label: 'Neutral-atom FT demo', slug: 'guides/na-ft-demo' },
						{ label: 'Maturation path', slug: 'guides/roadmap' },
						{ label: 'Application demos', slug: 'guides/applications' },
						{ label: 'Creative & games', slug: 'guides/creative' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'quonc CLI', slug: 'reference/quonc' },
						{ label: 'Compiler pipeline', slug: 'reference/compiler' },
					],
				},
				{
					label: 'Contributing',
					items: [
						{ label: 'Documenting Quon', slug: 'contribute' },
						{ label: 'Style guide', slug: 'contribute/style-guide' },
					],
				},
			],
		}),
	],
});
