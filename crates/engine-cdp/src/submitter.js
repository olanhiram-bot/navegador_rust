function () {
  // El botón que un Enter en este formulario pulsaría, con el mismo cssPath
  // que snapshot.js para que la ref del snapshot coincida exactamente.
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

  const el = document.activeElement;
  if (!el || !el.form) return JSON.stringify({ selector: null });
  const tag = el.tagName.toLowerCase();
  // En un textarea o en algo editable, Enter escribe una línea: no envía.
  if (tag === 'textarea' || el.isContentEditable) {
    return JSON.stringify({ selector: null });
  }

  const form = el.form;
  const candidates = Array.from(form.elements).filter((node) => {
    const tag = node.tagName.toLowerCase();
    const type = (node.getAttribute('type') || '').toLowerCase();
    if (node.disabled) return false;
    if (tag === 'button') return type !== 'button' && type !== 'reset';
    if (tag === 'input') return type === 'submit' || type === 'image';
    return false;
  });
  const submitter = candidates[0];
  if (!submitter) return JSON.stringify({ selector: null });
  return JSON.stringify({ selector: cssPath(submitter) });
}
