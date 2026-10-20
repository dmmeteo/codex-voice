class Element {
  constructor(attributes = {}) {
    Object.assign(this, {value: '', textContent: '', hidden: false, disabled: false, checked: false, dataset: {}, children: [], listeners: {}}, attributes);
  }
  addEventListener(type, listener) { this.listeners[type] = listener; }
  replaceChildren(...children) { this.children = children; }
  append(...children) { this.children.push(...children); }
  prepend(child) { this.children.unshift(child); }
  get lastChild() { const children = this.children; return {remove: () => children.pop()}; }
}

/** Elements with an id, carrying the attributes the controllers read. */
export function fakeDocument(html) {
  const elements = new Map();
  for (const [tag, name, id] of html.matchAll(/<(\w+)[^>]*\sid="([^"]+)"[^>]*>/g)) {
    const selected = name === 'select' ? html.slice(html.indexOf(tag)).match(/<option value="([^"]+)" selected>/)?.[1] : undefined;
    elements.set(id, new Element({
      checked: /\schecked(?:\s|>)/.test(tag), disabled: /\sdisabled(?:\s|>)/.test(tag), hidden: /\shidden(?:\s|>)/.test(tag),
      value: selected ?? '', files: name === 'input' && /type="file"/.test(tag) ? [] : undefined,
    }));
  }
  return {getElementById: id => elements.get(id), createElement: () => new Element()};
}
