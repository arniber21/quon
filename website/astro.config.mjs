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
					label: 'Guides',
					items: [{ label: 'Backends and verification', link: '/guides/backends/' }],
				},
			],
		}),
	],
});
