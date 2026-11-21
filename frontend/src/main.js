import { mount } from 'svelte';
import App from './App.svelte';
import Crossings from './Crossings.svelte';
import './app.css';

// One build serves two pages: nginx answers every path with index.html, so the
// path decides which screen mounts.
const path = window.location.pathname.replace(/\/+$/, '');
const page = path === '/crossings' ? Crossings : App;

const app = mount(page, { target: document.getElementById('app') });

export default app;
