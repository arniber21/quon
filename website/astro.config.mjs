// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
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
