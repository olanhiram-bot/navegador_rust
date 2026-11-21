function (nameOf) {
  const clean = (s) => (s || '').replace(/\s+/g, ' ').trim();
  const MAX_ELEMENTS = 120;

  const cssPath = (el) => {
    const uniqueId = (node) => {
      if (!node.id) return null;
      try {
        const matches = document.querySelectorAll('#' + CSS.escape(node.id));
        return matches.length === 1 ? '#' + CSS.escape(node.id) : null;
      } catch (e) {
        return null;
      }
    };

    const direct = uniqueId(el);
    if (direct) return direct;

    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && node !== document.documentElement) {
      const id = uniqueId(node);
      if (id) {
        parts.unshift(id);
        break;
      }
      const tag = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (!parent) {
        parts.unshift(tag);
        break;
      }
      const sameTag = Array.from(parent.children).filter(
        (child) => child.tagName === node.tagName
      );
      parts.unshift(
        sameTag.length === 1
          ? tag
          : tag + ':nth-of-type(' + (sameTag.indexOf(node) + 1) + ')'
      );
      node = parent;
    }
    return parts.join(' > ');
  };

  const roleOf = (el) => {
    const explicit = (el.getAttribute('role') || '').toLowerCase();
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'a') return 'link';
    if (tag === 'select') return 'select';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'summary') return 'button';
    if (tag === 'button') return 'button';
    if (tag === 'input') {
      if (type === 'checkbox') return 'checkbox';
      if (type === 'radio') return 'radio';
      if (type === 'file') return 'file';
      if (['submit', 'button', 'reset', 'image'].includes(type)) return 'button';
      return 'textbox';
    }
    return 'clickable';
  };

  const visible = (el) => {
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return false;
    const style = window.getComputedStyle(el);
    return style.visibility !== 'hidden' && style.display !== 'none';
  };

  const candidates = document.querySelectorAll(
    'a[href], button, input:not([type=hidden]), select, textarea, summary, ' +
      '[role=button], [role=link], [onclick]'
  );

  const elements = [];
  for (const el of Array.from(candidates)) {
    if (elements.length >= MAX_ELEMENTS) break;
    if (!visible(el)) continue;
    const rect = el.getBoundingClientRect();
    elements.push({
      selector: cssPath(el),
      role: roleOf(el),
      name: nameOf(el),
      tag: el.tagName.toUpperCase(),
      x: rect.x + rect.width / 2,
      y: rect.y + rect.height / 2,
      disabled: Boolean(el.disabled) || el.getAttribute('aria-disabled') === 'true',
    });
  }

  return JSON.stringify({
    url: location.href,
    title: document.title || '',
    text: clean(document.body ? document.body.innerText : '').slice(0, 4000),
    elements,
  });
}
