function (el) {
  const clean = (s) => (s || "").replace(/\s+/g, " ").trim();
  const MAX_NAME = 120;
  const aria = el.getAttribute('aria-label');
  if (aria) return clean(aria).slice(0, MAX_NAME);
  const labelledby = el.getAttribute('aria-labelledby');
  if (labelledby) {
    const target = document.getElementById(labelledby);
    if (target) return clean(target.innerText).slice(0, MAX_NAME);
  }
  if (el.labels && el.labels.length) {
    return clean(el.labels[0].innerText).slice(0, MAX_NAME);
  }
  const tag = el.tagName.toLowerCase();
  const type = (el.getAttribute('type') || '').toLowerCase();
  if (tag === 'input' && ['submit', 'button', 'reset'].includes(type)) {
    return clean(el.value).slice(0, MAX_NAME);
  }
  return clean(
    el.innerText || el.value || el.placeholder || el.title || el.name || ''
  ).slice(0, MAX_NAME);
}
