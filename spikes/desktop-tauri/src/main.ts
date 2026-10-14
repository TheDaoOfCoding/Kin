// The only UI in the spike: a greeting. The real windows (pairing, history, settings) go here.
const app = document.querySelector<HTMLElement>("#app");
if (app) app.textContent = "Hello from Kin Relay";
