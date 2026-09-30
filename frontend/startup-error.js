// Load before the application module so import failures remain visible in the window.
window.reportFrontendError = (error) => {
  const status = document.querySelector("#status");
  if (status) status.textContent = `Frontend error: ${String(error)}`;
};

window.addEventListener("error", (event) => {
  window.reportFrontendError(event.error?.message || event.message);
});
window.addEventListener("unhandledrejection", (event) => {
  window.reportFrontendError(event.reason);
});
