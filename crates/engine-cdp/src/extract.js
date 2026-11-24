function (selector, nameOf) {
  const el = document.querySelector(selector);
  if (!el) {
    return JSON.stringify({ error: 'selector_not_found', selector });
  }

  const clean = (s) => (s || '').replace(/\s+/g, ' ').trim();
  const form = el.form || el.closest('form');
  const rect = el.getBoundingClientRect();
  const tag = el.tagName.toUpperCase();
  const type = (el.getAttribute('type') || '').toLowerCase();

  const label = el.closest('label');
  const container = (form && form.parentElement) || el.parentElement || document.body;
  const visibleText = clean(
    [
      el.innerText,
      el.getAttribute('aria-label'),
      label && label.innerText,
      container && container.innerText,
    ]
      .filter(Boolean)
      .join(' | ')
  ).slice(0, 600);

  const fields = Object.create(null);
  let actionUrl = null;
  let method = null;
  let file = null;

  if (form) {
    // Los controles con nombre pueden ocultar propiedades de la instancia.
    const formProperty = (key) => Reflect.get(HTMLFormElement.prototype, key, form);
    actionUrl = formProperty('action') || location.href;
    method = formProperty('method').toUpperCase();
    for (const field of Array.from(formProperty('elements'))) {
      if (!field.name) continue;
      const fieldType = (field.type || '').toLowerCase();
      if (['submit', 'button', 'reset', 'image'].includes(fieldType)) continue;
      if ((fieldType === 'checkbox' || fieldType === 'radio') && !field.checked) continue;
      if (fieldType === 'file') {
        file = (field.files && field.files[0] && field.files[0].name) || null;
        continue;
      }
      fields[field.name] = field.value;
    }
  } else if (tag === 'A' && el.href) {
    actionUrl = el.href;
    method = 'GET';
  }

  let verb = 'click';
  if (tag === 'A') {
    verb = 'navigate';
  } else if (form) {
    if (tag === 'BUTTON' && (type === '' || type === 'submit')) verb = 'submit';
    if (tag === 'INPUT' && (type === 'submit' || type === 'image')) verb = 'submit';
  }

  const findKey = (re) => Object.keys(fields).find((key) => re.test(key));
  const amountKey =
    findKey(/amount|total|price|importe|monto|sum/i) ||
    findKey(/value/i);
  const recipientKey =
    findKey(/^(to|recipient|dest|destination|payee|account|iban|email)$/i) ||
    findKey(/recipient|payee|iban|account|email/i);

  const number = (value) => {
    if (value === null || value === undefined) return null;
    const cleaned = String(value).replace(/[^0-9.,-]/g, '').replace(',', '.');
    const parsed = parseFloat(cleaned);
    return Number.isFinite(parsed) ? parsed : null;
  };

  return JSON.stringify({
    verb,
    document_origin: location.origin,
    document_url: location.href,
    action_url: actionUrl,
    method,
    fields,
    amount: amountKey ? number(fields[amountKey]) : null,
    recipient: recipientKey ? fields[recipientKey] : null,
    file,
    visible_text: visibleText,
    target: {
      selector,
      tag,
      name: nameOf(el),
      x: rect.x + rect.width / 2,
      y: rect.y + rect.height / 2,
    },
  });
}
