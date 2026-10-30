// Browser input is the public contacts array, never the private enrollment file.
export function mountContacts(container, contacts, selected) {
  if (!Array.isArray(contacts) || contacts.length > 3 ||
      typeof selected !== 'function') throw new TypeError('Invalid contact directory');
  const list = document.createElement('div');
  list.setAttribute('aria-label', 'Contacts');
  for (const contact of contacts) {
    if (!contact || typeof contact.key !== 'string' ||
        typeof contact.label !== 'string' || typeof contact.identityId !== 'string')
      throw new TypeError('Invalid contact');
    const captured = Object.freeze({...contact});
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = captured.label;
    button.addEventListener('click', () => selected(captured));
    list.append(button);
  }
  container.append(list);
  return () => list.remove();
}
