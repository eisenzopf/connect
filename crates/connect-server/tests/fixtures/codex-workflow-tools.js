// Code-mode adapter used only by the scripted local runtime qualification.
async function invoke(name, args) {
  const candidates = ALL_TOOLS.filter(t => t.name.endsWith(name) && t.name.includes('connect_task'));
  if (candidates.length !== 1) throw new Error('Expected one installed ' + name);
  const result = await tools[candidates[0].name](args);
  if (result.isError || result.is_error) throw new Error('Connect rejected ' + name);
  if (result.structuredContent) return result.structuredContent;
  if (result.structured_content) return result.structured_content;
  return JSON.parse(result.content.find(item => item.type === 'text').text);
}
