const assert = require('node:assert/strict');
const vm = require('node:vm');
const { page, english, chinese, languages: bundledLanguages } = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
const script = page.split('<script>')[1].split('</script>')[0];
const languagePopup = {};
const initialLanguageCombo = { querySelector: () => languagePopup };
const document = {
  documentElement: { lang: 'en' },
  querySelectorAll: () => [],
  getElementById: id => id === 'cb-appearance-language' ? initialLanguageCombo : null,
  addEventListener: () => {}
};
const context = vm.createContext({ document, window: { _localeStrings: english, _language: 'en', _languages: bundledLanguages } });
vm.runInContext(script, context);
const readPresetEntries = context.readPresetEntries;
const languageMarkup = languagePopup.innerHTML;
assert.equal((languageMarkup.match(/class="combobox-option"/g) || []).length, bundledLanguages.length);
for (const language of bundledLanguages) {
  assert.ok(languageMarkup.includes(`data-value="${context.escAttr(language.identifier)}">${context.escHtml(language.name)}</div>`), language.identifier);
}
const hostileLanguage = { identifier: 'x-"<&', name: '<b>&"Native</b>' };
assert.equal(context.languageOptionsMarkup([hostileLanguage]),
  '<div class="combobox-option" data-value="x-&quot;&lt;&amp;">&lt;b&gt;&amp;&quot;Native&lt;/b&gt;</div>');

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

function attributeKeys(markup) {
  return Array.from(markup.matchAll(/\bdata-i18n(?:-[a-z-]+)?="([^"]+)"/g), match => match[1]);
}
function assertKnownKeys(markup, catalog = english) {
  for (const key of attributeKeys(markup)) {
    assert.ok(Object.hasOwn(catalog, key), `Unknown generated locale key: ${key}`);
  }
}
assertKnownKeys(page.split('<script>')[0]);

function ruleFixture(source, rule = {}) {
  const fields = {};
  const shell = { addEventListener() {}, querySelectorAll: () => [], classList: { contains: () => false } };
  const row = {
    querySelectorAll: () => [],
    querySelector: selector => fields[selector] || (['.rule-opts-btn', '.rule-opts-pop', '.rule-maximized', '.rule-sticky'].includes(selector)
      ? shell : null)
  };
  const doc = {
    documentElement: { lang: 'en' }, addEventListener() {}, querySelectorAll: () => [],
    getElementById: () => null
  };
  const ctx = vm.createContext({ document: doc, window: { _localeStrings: english } });
  vm.runInContext(source, ctx);
  doc.createElement = () => row;
  doc.getElementById = () => ({ appendChild() {} });
  ctx.addRuleRow(rule);
  for (const match of row.innerHTML.matchAll(/<input\b[^>]*class="([^"]+)"[^>]*value="([^"]*)"/g)) {
    fields['.' + match[1].split(' ')[0]] = { value: match[2] };
  }
  fields['.rule-action'] = { dataset: { value: rule.action || 'tile' } };
  fields['.rule-corner'] = { dataset: { value: 'auto' } };
  fields['.rule-workspace'] = { dataset: { value: '' } };
  fields['.rule-opts-summary'] = {};
  doc.querySelectorAll = selector => selector === '#rules-body tr' ? [row] : [];
  return { ctx, row, fields };
}
const rules = ruleFixture(script).row.innerHTML;
assertKnownKeys(rules);
const ruleKeys = new Set(attributeKeys(rules));
assert.ok(ruleKeys.size > 0);
for (const key of ruleKeys) {
  const incomplete = { ...english };
  delete incomplete[key];
  assert.throws(() => assertKnownKeys(rules, incomplete), error =>
    error.message.includes(`Unknown generated locale key: ${key}`));
}

const sizeRule = { match_executable: 'QQ.exe', action: 'ignore', match_max_width: 400, match_max_height: 300, width: 900, height: 700 };
const sizeFixture = ruleFixture(script, sizeRule);
const savedRules = () => JSON.parse(JSON.stringify(sizeFixture.ctx.readRules()));
assert.deepEqual(savedRules(), [sizeRule]);
sizeFixture.ctx.updateRuleSummary(sizeFixture.row);
assert.equal(sizeFixture.fields['.rule-opts-summary'].textContent, 'Max width 400 · Max height 300');
sizeFixture.ctx.localeStrings = chinese;
sizeFixture.ctx.updateRuleSummary(sizeFixture.row);
assert.equal(sizeFixture.fields['.rule-opts-summary'].textContent, '最大宽度 400 · 最大高度 300');
for (const [width, height, expected] of [
  ['500', '350', { match_max_width: 500, match_max_height: 350 }],
  ['', '', {}],
  ['0', '-1', {}],
  ['1.5', 'invalid', {}],
  ['1', '', { match_max_width: 1 }],
  ['', '1', { match_max_height: 1 }]
]) {
  sizeFixture.fields['.rule-max-width'].value = width;
  sizeFixture.fields['.rule-max-height'].value = height;
  assert.deepEqual(savedRules(), [{ match_executable: 'QQ.exe', action: 'ignore', width: 900, height: 700, ...expected }]);
}
assert.equal(sizeRule.match_max_width, 400);
assert.equal(sizeRule.match_max_height, 300);
sizeFixture.fields['.rule-exe'].value = '';
assert.deepEqual(savedRules(), []);

function eventElement() {
  const listeners = {};
  const classes = new Set();
  return {
    value: '', checked: false, dataset: {}, style: {}, textContent: '',
    classList: {
      add: name => classes.add(name), remove: name => classes.delete(name),
      contains: name => classes.has(name)
    },
    addEventListener(name, callback) { (listeners[name] ||= []).push(callback); },
    emit(name) { for (const callback of listeners[name] || []) callback({ stopPropagation() {} }); }
  };
}
function presetFixture(values = [0.333, 0.5, 0.667]) {
  const rows = [];
  const fields = {};
  const timers = new Map();
  const saves = [];
  let nextTimer = 0;
  const doc = {
    documentElement: { lang: 'en' }, addEventListener() {}, querySelectorAll: () => [],
    getElementById: () => null, querySelector: () => null
  };
  const ctx = vm.createContext({
    document: doc, window: { _localeStrings: english, ipc: { postMessage: json => saves.push(JSON.parse(json)) } },
    setTimeout(callback, delay) { timers.set(++nextTimer, { callback, delay }); return nextTimer; },
    clearTimeout(id) { timers.delete(id); }
  });
  vm.runInContext(script, ctx);
  const trigger = eventElement();
  const triggerText = eventElement();
  const popup = eventElement();
  const combo = eventElement();
  let options = [];
  Object.defineProperty(combo, 'innerHTML', {
    set(markup) {
      triggerText.textContent = markup.match(/class="combobox-text">([^<]*)/)[1];
      options = Array.from(markup.matchAll(/class="combobox-option([^"]*)" data-value="([^"]+)">([^<]*)/g), match => {
        const option = eventElement();
        if (match[1].includes('selected')) option.classList.add('selected');
        option.dataset.value = match[2]; option.textContent = match[3];
        return option;
      });
    }
  });
  combo.querySelectorAll = () => options;
  combo.querySelector = selector => selector === '.combobox-trigger' ? trigger
    : selector === '.combobox-text' ? triggerText : popup;
  const language = eventElement();
  const languageLabel = eventElement();
  const languageOptions = [['en', 'English'], ['zh-CN', '简体中文']].map(([value, textContent]) => {
    const option = eventElement(); option.dataset.value = value; option.textContent = textContent; return option;
  });
  language.querySelectorAll = () => languageOptions;
  language.querySelector = () => languageLabel;
  fields['cb-layout-default_width_preset'] = combo;
  fields['cb-appearance-language'] = language;
  fields['width-presets-body'] = { appendChild: row => rows.push(row) };
  doc.getElementById = id => fields[id] ||= eventElement();
  doc.querySelectorAll = selector => selector === '#width-presets-body tr' ? rows
    : selector === '.combobox' ? [combo, language] : [];
  doc.createElement = () => {
    const input = eventElement();
    const button = eventElement();
    const row = {
      isConnected: true, input, querySelectorAll: () => [],
      querySelector: selector => selector === '.row-delete' ? button : input
    };
    Object.defineProperty(row, 'innerHTML', { set(markup) { input.value = markup.match(/value="([^"]*)"/)[1]; } });
    return row;
  };
  values.forEach(value => ctx.addPresetRow('width', value));
  ctx.refreshDefaultWidthPresetOptions(1);
  return { ctx, rows, combo, options: () => options, timers, saves,
    flush() { const pending = Array.from(timers.values()); timers.clear(); pending.forEach(timer => timer.callback()); } };
}
function presetState(fixture) {
  const { ctx, combo } = fixture;
  return {
    selected: ctx.selectedWidthPresetRow,
    value: combo.dataset.value,
    widths: Array.from(ctx.lastValidWidthPresets),
    defaultPreset: ctx.lastValidDefaultWidthPreset
  };
}
const pending = presetFixture();
pending.options()[1].emit('click');
assert.equal(pending.timers.size, 1);
assert.equal(Array.from(pending.timers.values())[0].delay, 0);
assert.equal(pending.ctx.selectedWidthPresetRow, pending.rows[1]);
const selectedBeforePush = presetState(pending);
pending.ctx.refreshLocale(chinese, 'zh-CN');
assert.deepEqual(presetState(pending), selectedBeforePush);
pending.flush();
assert.equal(pending.saves[0].config.layout.default_width_preset, 2);
assert.deepEqual(pending.saves[0].config.layout.width_presets, [0.333, 0.5, 0.667]);

for (const [draft, expectedWidths, expectedDefault] of [
  ['0.6', [0.333, 0.6, 0.667], 2],
  ['unfinished', [0.333, 0.667], 1],
  ['', [0.333, 0.667], 1]
]) {
  const fixture = presetFixture();
  fixture.options()[1].emit('click');
  fixture.flush();
  fixture.rows[1].input.value = draft;
  fixture.rows[1].input.emit('input');
  assert.equal(fixture.timers.size, 1);
  assert.equal(Array.from(fixture.timers.values())[0].delay, 500);
  const beforePush = presetState(fixture);
  fixture.ctx.refreshLocale(chinese, 'zh-CN');
  assert.deepEqual(presetState(fixture), beforePush);
  assert.equal(fixture.rows[1].input.value, draft);
  fixture.flush();
  assert.equal(fixture.saves[1].config.layout.default_width_preset, expectedDefault);
  assert.deepEqual(fixture.saves[1].config.layout.width_presets, expectedWidths);
}
const invalid = presetFixture([0.5]);
invalid.rows[0].input.value = 'unfinished';
invalid.rows[0].input.emit('input');
assert.equal(Array.from(invalid.timers.values())[0].delay, 500);
const fallbackBeforePush = presetState(invalid);
invalid.ctx.refreshLocale(chinese, 'zh-CN');
assert.deepEqual(presetState(invalid), fallbackBeforePush);
assert.equal(invalid.rows[0].input.value, 'unfinished');
invalid.flush();
assert.deepEqual(invalid.saves[0].config.layout.width_presets, [0.5]);
assert.equal(invalid.saves[0].config.layout.default_width_preset, 1);
