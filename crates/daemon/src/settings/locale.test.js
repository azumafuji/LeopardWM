const assert = require('node:assert/strict');
const vm = require('node:vm');
const { page, english, chinese } = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
const script = page.split('<script>')[1].split('</script>')[0];
const document = {
  documentElement: { lang: 'en' },
  querySelectorAll: () => [],
  getElementById: () => null,
  addEventListener: () => {}
};
const context = vm.createContext({ document, window: { _localeStrings: english, _language: 'en' } });
vm.runInContext(script, context);
const readPresetEntries = context.readPresetEntries;

const hostile = '<b>&"中文</b>';
context.localeStrings = {
  'settings.text.preset_index_percent': hostile + ' {index} ({percent}%)'
};
const row = {};
const trigger = {};
const cb = {
  dataset: {},
  classList: { remove: () => {}, add: () => {} },
  querySelector: () => trigger,
  querySelectorAll: () => []
};
document.getElementById = () => cb;
context.readPresetEntries = () => [{ row, value: 0.5 }];
context.selectedWidthPresetRow = row;
context.initCombobox = () => {};
context.refreshDefaultWidthPresetOptions();
assert.ok(cb.innerHTML.includes('&lt;b&gt;&amp;&quot;中文&lt;/b&gt; 1 (50%)'));
assert.ok(!cb.innerHTML.includes(hostile));
assert.equal(cb.dataset.value, '1');

const text = { dataset: { i18n: 'settings.text.settings' }, textContent: '' };
Object.defineProperty(text, 'innerHTML', { set: () => { throw new Error('Raw HTML translation'); } });
const attribute = {
  attributes: {},
  getAttribute: () => 'settings.text.settings',
  setAttribute(name, value) { this.attributes[name] = value; }
};
document.querySelectorAll = selector => selector === '[data-i18n]' ? [text]
  : selector.startsWith('[data-i18n-') ? [attribute] : [];
context.applyLocale({ 'settings.text.settings': hostile }, 'zh-CN');
assert.equal(text.textContent, hostile);
assert.equal(attribute.attributes.title, hostile);
assert.equal(attribute.attributes['aria-label'], hostile);
assert.equal(attribute.attributes.placeholder, hostile);
assert.equal(document.documentElement.lang, 'zh-CN');

const draft = { value: 'unfinished preset' };
const deleteButton = {};
row.isConnected = true;
row.querySelector = selector => selector === '.preset-val' ? draft : deleteButton;
const languageText = {};
const languages = [['en', 'English'], ['zh-CN', '简体中文']].map(([value, textContent]) => ({
  dataset: { value }, textContent, classList: { add: () => {}, remove: () => {} }
}));
const languageCombo = {
  dataset: { value: 'en' },
  querySelector: () => languageText,
  querySelectorAll: () => languages
};
document.querySelector = () => null;
document.querySelectorAll = selector => selector === '[data-i18n]' ? [text]
  : selector.startsWith('[data-i18n-') ? [attribute]
  : selector === '#width-presets-body tr' ? [row]
  : selector === '.combobox' ? [languageCombo] : [];
document.getElementById = id => id === 'cb-layout-default_width_preset' ? cb
  : id === 'cb-appearance-language' ? languageCombo : null;
context.readPresetEntries = readPresetEntries;
context.refreshLocale(english, 'en');
assert.equal(text.textContent, 'Settings');
context.refreshLocale(chinese, 'zh-CN');
assert.equal(text.textContent, '设置');
assert.equal(languageText.textContent, '简体中文');
assert.equal(languageCombo.dataset.value, 'zh-CN');
assert.equal(draft.value, 'unfinished preset');
assert.equal(context.selectedWidthPresetRow, row);
context.refreshLocale(english, 'en');
assert.equal(text.textContent, 'Settings');
assert.equal(languageText.textContent, 'English');
assert.equal(draft.value, 'unfinished preset');
const warnings = Object.fromEntries(['hotkey-warn-bar', 'hotkey-warn-title', 'hotkey-warn-msg']
  .map(id => [id, {}]));
document.getElementById = id => warnings[id];
context.localeStrings = english;
for (const [keys, title, combos, verb] of [
  [['Win+L'], '1 hotkey is likely unsupported.', 'Win+L', 'is'],
  [['Win+L', 'Ctrl+Alt+Del'], '2 hotkeys are likely unsupported.', 'Win+L and Ctrl+Alt+Del', 'are'],
  [['A', 'B', 'C'], '3 hotkeys are likely unsupported.', 'A, B, and C', 'are']
]) {
  context.window._failedHotkeys = keys;
  context.renderFailedHotkeys();
  assert.equal(warnings['hotkey-warn-title'].textContent, title);
  assert.equal(warnings['hotkey-warn-msg'].textContent,
    `${combos} ${verb} reserved by Windows and can't be intercepted, so it likely won't fire. Pick a different combination.`);
  assert.equal(warnings['hotkey-warn-bar'].hidden, false);
}
context.localeStrings = chinese;
context.window._failedHotkeys = ['Win+L'];
context.renderFailedHotkeys();
assert.equal(warnings['hotkey-warn-title'].textContent, '1 个快捷键可能不受支持。');
assert.ok(warnings['hotkey-warn-msg'].textContent.startsWith('Win+L 已被 Windows 保留'));
context.window._failedHotkeys = [];
context.renderFailedHotkeys();
assert.equal(warnings['hotkey-warn-bar'].hidden, true);
context.localeStrings = { test: '{name} {count} {unknown}' };
assert.equal(context.t('test', { name: '{count}', count: 2 }), '{count} 2 {unknown}');
