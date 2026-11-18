// The docs pages' VitePress theme: the default theme without its fonts, in the main page's look: the
// web UI's tokens and bundled fonts (site/main/main.js), and the main page's header, its logo before
// the title. site/config.mjs gives the header's links; build.mjs stages this file as the theme.
import '@fontsource-variable/inter';
import '@fontsource-variable/space-grotesk';
import '@fontsource-variable/jetbrains-mono';
import DefaultTheme from 'vitepress/theme-without-fonts';
import { h } from 'vue';
import '../../web/src/theme/theme.css';
import './docs.css';

// The main page's logo, site/main/index.html.
const logo = () =>
  h('svg', { class: 'skald-logo', width: 28, height: 28, viewBox: '0 0 28 28', 'aria-hidden': 'true' }, [
    h('rect', { width: 28, height: 28, rx: 7, fill: 'var(--color-accent)' }),
    h('path', {
      d: 'M10 7 L18 11.5 L10 16.5 L18 21',
      fill: 'none',
      stroke: 'var(--color-accent-text)',
      'stroke-width': 2.4,
      'stroke-linecap': 'round',
      'stroke-linejoin': 'round',
    }),
  ]);

export default {
  extends: DefaultTheme,
  Layout: () => h(DefaultTheme.Layout, null, { 'nav-bar-title-before': logo }),
};
