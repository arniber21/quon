// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	site: 'https://quon.arnabg.me',
	integrations: [
		starlight({
			title: 'quon',
			sidebar: [
				{
					label: 'Getting Started',
					items: [
						{ label: 'Install Quon', slug: 'getting-started/install' },
						{ label: 'Quickstart', slug: 'getting-started/quickstart' },
					],
				},
				{
					label: 'Language guide',
					items: [{ label: 'Language fundamentals', link: '/language/' }],
				},
				{
					label: 'Cookbook',
					items: [{ autogenerate: { directory: 'cookbook' } }],
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
