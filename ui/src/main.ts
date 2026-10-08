import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { setLocale } from './lib/i18n';

setLocale(navigator.language || 'en');

const target = document.getElementById('app');
if (!target) throw new Error('#app element missing');

export default mount(App, { target });
