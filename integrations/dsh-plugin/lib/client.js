// Browser half of @dane/dsh-web-search-tokenloom: the Plugins settings card.
//
// The Host-side plugin (index.js) installs the `web-search-tokenloom` settings
// section; this half claims it in Settings → Plugins → "Plugin configuration"
// by registering a card into the `settings.plugin.item` slot, keyed by the
// namespace. The tab dispatches by namespace, so without this file the served
// namespace renders nothing.
//
// The form stages edits and writes them only on save — each write is a
// durable, revision-fenced settings mutation (scope.set / scope.unset), the
// same contract the built-in Shell / Agent loop cards use.
//
// Visual structure below mirrors the Host's own PluginCard/ValueField
// (packages/client/ui-settings-plugins) so this card sits visually flush with
// Shell and Agent loop. Those components aren't exported by the Host bundle,
// so this reproduces their DOM shape and reads the same --dsw-alias-* theme
// tokens rather than importing them.

window.__ModuleLoader__.load({
  id: "@dane/dsh-web-search-tokenloom",
  factory: (require) => {
    var react_jsx_runtime = require("react/jsx-runtime");
    var jsxRuntime = react_jsx_runtime;
    var react = require("react");
    var module = { exports: {} };
    var exports = module.exports;

    /** Settings namespace owned by the Host-side plugin. */
    var NS = "web-search-tokenloom";

    //#region field specs
    function numberField(field) {
      return {
        field,
        format: (value) => typeof value === "number" ? String(value) : "",
        parse: (text) => {
          var trimmed = text.trim();
          if (trimmed === "") return { kind: "clear" };
          var parsed = Number(trimmed);
          return Number.isFinite(parsed) ? { kind: "set", value: parsed } : void 0;
        }
      };
    }
    function textField(field) {
      return {
        field,
        format: (value) => typeof value === "string" ? value : "",
        parse: (text) => {
          var trimmed = text.trim();
          return trimmed === "" ? { kind: "clear" } : { kind: "set", value: trimmed };
        }
      };
    }
    //#endregion

    //#region staged form over the namespace scope
    var CardForm = class {
      constructor(scope, specs) {
        this.scope = scope;
        this.specs = new Map(specs.map((spec) => [spec.field, spec]));
        this.staged = new Map();
        this.listeners = new Set();
        this.saving = false;
        this.failed = false;
        scope.subscribe(() => this.publish());
      }
      bind(project) {
        var store = { value: project(), listeners: new Set() };
        store.getSnapshot = function() { return store.value; };
        store.subscribe = function(listener) {
          store.listeners.add(listener);
          return function() { store.listeners.delete(listener); };
        };
        store.set = function(next) {
          store.value = next;
          for (var l of store.listeners) l();
        };
        this.listeners.add(function() { store.set(project()); });
        return store;
      }
      shell() {
        var snapshot = this.scope.getSnapshot();
        var plan = this.plan();
        return {
          available: snapshot.status === "ready",
          writable: snapshot.writable,
          dirty: plan.length > 0,
          invalid: plan.some(function(item) { return item.run === void 0; }),
          saving: this.saving,
          failed: this.failed
        };
      }
      field(field) {
        var staged = this.staged.get(field);
        var spec = this.specs.get(field);
        if (staged === void 0) return {
          text: spec.format(this.sectionValue(field)),
          overridden: this.stored(field),
          invalid: false
        };
        var write = staged.clear ? { kind: "clear" } : spec.parse(staged.text);
        return {
          text: staged.text,
          overridden: write !== void 0 && write.kind === "set",
          invalid: write === void 0
        };
      }
      actions() {
        var self = this;
        return {
          edit: function(field, text) { self.stage(field, { text, clear: false }); },
          resetField: function(field) {
            self.stage(field, { text: self.specs.get(field).format(self.baseValue(field)), clear: true });
          },
          save: function() { self.save(); },
          discard: function() {
            if (self.staged.size === 0 && !self.failed) return;
            self.staged.clear();
            self.failed = false;
            self.publish();
          }
        };
      }
      async save() {
        var plan = this.plan();
        var writes = [];
        for (var item of plan) if (item.run !== void 0) writes.push(item.run);
        if (plan.length === 0 || this.saving || writes.length !== plan.length) return;
        this.saving = true;
        this.failed = false;
        this.publish();
        var landed = true;
        for (var run of writes) {
          try { landed = (await run()) && landed; }
          catch (_writeFailure) { landed = false; }
        }
        if (landed) this.staged.clear();
        this.saving = false;
        this.failed = !landed;
        this.publish();
      }
      plan() {
        var plan = [];
        for (var entry of this.staged) {
          var field = entry[0], staged = entry[1];
          var spec = this.specs.get(field);
          if (staged.clear) {
            if (this.stored(field)) plan.push({ field, run: () => this.runWrite(async () => { await this.scope.unset(field); }) });
            continue;
          }
          if (staged.text === spec.format(this.sectionValue(field))) continue;
          var write = spec.parse(staged.text);
          if (write === void 0) plan.push({ field, run: void 0 });
          else if (write.kind === "clear") plan.push({ field, run: () => this.runWrite(async () => { await this.scope.unset(field); }) });
          else plan.push({ field, run: () => this.runWrite(async () => { await this.scope.set(field, write.value); }) });
        }
        return plan;
      }
      // Wrap one write and report whether the Host accepted it (user layer reflects the value).
      async runWrite(commit) {
        try { await commit(); } catch (_rejected) { return false; }
        return true;
      }
      stage(field, edit) {
        this.staged.set(field, edit);
        this.failed = false;
        this.publish();
      }
      snapshotOf() { return this.scope.getSnapshot(); }
      sectionValue(field) { return this.snapshotOf().value?.[field]; }
      baseValue(field) { return this.snapshotOf().base?.[field]; }
      stored(field) {
        var user = this.snapshotOf().user;
        return user !== void 0 && Object.hasOwn(user, field);
      }
      publish() {
        for (var l of this.listeners) l();
      }
    };
    //#endregion

    //#region controller
    var TokenloomCardController = class {
      constructor(scope) {
        var controller = this;
        this.form = new CardForm(scope, [
          textField("bin"),
          numberField("maxResults"),
          numberField("timeoutMs"),
          numberField("fetchTimeoutMs")
        ]);
        this.store = this.form.bind(function() { return controller.projection(); });
      }
      projection() {
        return {
          ...this.form.shell(),
          bin: this.form.field("bin"),
          maxResults: this.form.field("maxResults"),
          timeoutMs: this.form.field("timeoutMs"),
          fetchTimeoutMs: this.form.field("fetchTimeoutMs")
        };
      }
      inject() {
        return {
          hooks: { tokenloomCard: this.store },
          ...this.form.actions()
        };
      }
    };
    //#endregion

    //#region theme tokens
    // Same custom properties the Host's own PluginCard/ValueField reference
    // (packages/client/ui-settings-plugins/src/client/{PluginCard,fields}.module.css).
    // Reproduced as inline styles here since those modules aren't exported for
    // plugins to import — this keeps the card on the live theme without
    // depending on the Host's internal, unversioned class names.
    var v = {
      borderL2: "var(--dsw-alias-border-l2)",
      bgLayer2: "var(--dsw-alias-bg-layer-2)",
      bgLayer3: "var(--dsw-alias-bg-layer-3)",
      bgModulePlatform: "var(--dsw-alias-bg-module-platform)",
      labelPrimary: "var(--dsw-alias-label-primary)",
      labelSecondary: "var(--dsw-alias-label-secondary)",
      labelTertiary: "var(--dsw-alias-label-tertiary)",
      labelDimmed: "var(--dsw-alias-label-dimmed)",
      labelError: "var(--dsw-alias-label-error)",
      brandPrimary: "var(--dsw-alias-brand-primary)"
    };

    var styles = {
      card: (open) => ({
        border: "1px solid " + (open ? v.labelDimmed : v.borderL2),
        background: open ? v.bgLayer2 : v.bgLayer3,
        borderRadius: "12px",
        listStyle: "none",
        transition: "border-color .16s, background .16s"
      }),
      header: {
        appearance: "none",
        width: "100%",
        font: "inherit",
        color: "inherit",
        textAlign: "left",
        cursor: "pointer",
        background: "transparent",
        border: 0,
        borderRadius: "12px",
        display: "flex",
        alignItems: "center",
        gap: "12px",
        padding: "14px 16px"
      },
      headText: { display: "flex", flexDirection: "column", flex: 1, gap: "4px", minWidth: 0 },
      name: { color: v.labelPrimary, fontSize: "15px", fontWeight: 600, lineHeight: 1.4 },
      description: { color: v.labelTertiary, fontSize: "13px", lineHeight: 1.5 },
      chevron: (open) => ({ color: v.labelTertiary, flexShrink: 0, transition: "transform .16s", transform: open ? "rotate(180deg)" : "none" }),
      body: { borderTop: "1px solid " + v.borderL2, margin: "0 16px", paddingBottom: "8px" },
      readOnly: { color: v.labelTertiary, margin: "12px 0 0", fontSize: "12px", lineHeight: 1.5 },
      pending: { whiteSpace: "nowrap", background: v.bgModulePlatform, color: v.labelSecondary, borderRadius: "999px", flexShrink: 0, padding: "1px 8px", fontSize: "11px", fontWeight: 500, lineHeight: "17px" },
      footer: { borderTop: "1px solid " + v.borderL2, display: "flex", justifyContent: "flex-end", alignItems: "center", gap: "8px", padding: "12px 0 4px" },
      failed: { minWidth: 0, color: v.labelError, flex: 1, margin: 0, fontSize: "12px", lineHeight: 1.5 },
      buttonBase: { appearance: "none", font: "inherit", cursor: "pointer", border: "1px solid transparent", borderRadius: "8px", padding: "5px 14px", fontSize: "13px", lineHeight: 1.5 },
      discard: { borderColor: v.borderL2, color: v.labelSecondary, background: "transparent" },
      save: { background: v.labelPrimary, color: v.bgLayer3 },
      field: { display: "flex", flexDirection: "column", gap: "6px", padding: "12px 0", borderTop: "1px solid " + v.borderL2 },
      fieldFirst: { display: "flex", flexDirection: "column", gap: "6px", padding: "12px 0" },
      fieldHead: { display: "flex", alignItems: "center", gap: "8px" },
      label: { minWidth: 0, color: v.labelPrimary, flex: 1, fontSize: "13px", fontWeight: 500, lineHeight: 1.5 },
      badges: { display: "inline-flex", alignItems: "center", gap: "8px" },
      badge: { whiteSpace: "nowrap", background: v.bgModulePlatform, color: v.labelSecondary, borderRadius: "999px", padding: "1px 8px", fontSize: "11px", fontWeight: 500, lineHeight: "17px" },
      reset: { font: "inherit", color: v.labelSecondary, cursor: "pointer", background: "transparent", border: "none", padding: 0, fontSize: "12px", lineHeight: 1.5 },
      input: { border: "1px solid " + v.borderL2, background: v.bgLayer3, height: "34px", font: "inherit", color: v.labelPrimary, borderRadius: "8px", padding: "0 12px", fontSize: "13px", lineHeight: 1.5, width: "100%", boxSizing: "border-box" },
      inputInvalid: { borderColor: v.labelError },
      invalidText: { color: v.labelError, margin: 0, fontSize: "12px", lineHeight: 1.5 },
      hint: { color: v.labelTertiary, margin: 0, fontSize: "12px", lineHeight: 1.5 }
    };
    //#endregion

    //#region field component (mirrors the Host's ValueField)
    function ValueField(props) {
      return jsxRuntime.jsxs("div", {
        style: props.first ? styles.fieldFirst : styles.field,
        children: [
          jsxRuntime.jsxs("div", {
            style: styles.fieldHead, children: [
              jsxRuntime.jsx("label", { style: styles.label, htmlFor: props.id, children: props.label }),
              props.overridden ? jsxRuntime.jsxs("span", {
                style: styles.badges, children: [
                  jsxRuntime.jsx("span", { style: styles.badge, children: "Overridden" }),
                  jsxRuntime.jsx("button", { type: "button", style: styles.reset, disabled: props.disabled, onClick: props.onReset, children: "Reset to default" })
                ]
              }) : null
            ]
          }),
          jsxRuntime.jsx("input", {
            id: props.id,
            style: props.invalid ? { ...styles.input, ...styles.inputInvalid } : styles.input,
            type: "text",
            inputMode: props.numeric ? "numeric" : undefined,
            "aria-invalid": props.invalid ? true : undefined,
            disabled: props.disabled,
            value: props.text,
            onChange: function(event) { props.onEdit(event.target.value); }
          }),
          jsxRuntime.jsx("p", { style: props.invalid ? styles.invalidText : styles.hint, children: props.invalid ? props.invalidLabel : props.hint })
        ]
      });
    }
    //#endregion

    //#region card shell (mirrors the Host's PluginCard)
    var chevronPath = "M4.293 7.293a1 1 0 0 1 1.414 0L8 9.586l2.293-2.293a1 1 0 1 1 1.414 1.414l-3 3a1 1 0 0 1-1.414 0l-3-3a1 1 0 0 1 0-1.414z";
    function TokenloomCard(props) {
      var state = props.useTokenloomCard(function(snapshot) { return snapshot; });
      var openState = react.useState(false);
      var open = openState[0], setOpen = openState[1];
      if (!state.available) return null;
      var disabled = !state.writable || state.saving;
      var blocked = !state.dirty || state.invalid || state.saving;
      var fields = [
        { field: "bin", id: "plugin-config-tokenloom-bin", label: "Binary", hint: "Path or $PATH name of the tokenloom CLI. Leave blank to use the default.", numeric: false, first: true },
        { field: "maxResults", id: "plugin-config-tokenloom-max-results", label: "Max results per search", hint: "How many federated results one search returns.", numeric: true },
        { field: "timeoutMs", id: "plugin-config-tokenloom-search-timeout", label: "Search timeout (ms)", hint: "Wall-clock budget for one search invocation.", numeric: true },
        { field: "fetchTimeoutMs", id: "plugin-config-tokenloom-fetch-timeout", label: "Fetch timeout (ms)", hint: "Budget for one fetch; the SPA fallback ladder can run long.", numeric: true }
      ];
      return jsxRuntime.jsxs("li", {
        style: styles.card(open), children: [
          jsxRuntime.jsxs("button", {
            type: "button",
            style: styles.header,
            "aria-expanded": open,
            "aria-label": (open ? "Hide settings" : "Show settings") + ": tokenloom",
            onClick: function() { setOpen(!open); },
            children: [
              jsxRuntime.jsxs("span", {
                style: styles.headText, children: [
                  jsxRuntime.jsx("span", { style: styles.name, children: "tokenloom" }),
                  jsxRuntime.jsx("span", { style: styles.description, children: "Federated web search and 7-layer-sanitised page fetching." })
                ]
              }),
              state.dirty ? jsxRuntime.jsx("span", { style: styles.pending, children: "Unsaved" }) : null,
              jsxRuntime.jsx("svg", {
                style: styles.chevron(open),
                width: 14, height: 14, viewBox: "0 0 16 16", fill: "currentColor",
                "aria-hidden": true,
                children: jsxRuntime.jsx("path", { d: chevronPath })
              })
            ]
          }),
          open ? jsxRuntime.jsxs("div", {
            style: styles.body, children: [
              !state.writable ? jsxRuntime.jsx("p", { style: styles.readOnly, role: "status", children: "This deployment stores settings read-only." }) : null,
              fields.map(function(f) {
                return jsxRuntime.jsx(ValueField, {
                  id: f.id,
                  label: f.label,
                  hint: f.hint,
                  numeric: f.numeric,
                  first: f.first,
                  disabled: disabled,
                  ...state[f.field],
                  onEdit: function(text) { props.edit(f.field, text); },
                  onReset: function() { props.resetField(f.field); }
                }, f.field);
              }),
              jsxRuntime.jsxs("div", {
                style: styles.footer, children: [
                  state.failed ? jsxRuntime.jsx("p", { style: styles.failed, role: "status", children: "The deployment did not accept these values; they were left for you to correct." }) : null,
                  jsxRuntime.jsx("button", { type: "button", style: { ...styles.buttonBase, ...styles.discard }, disabled: !state.dirty || state.saving, onClick: props.discard, children: "Discard" }),
                  jsxRuntime.jsx("button", { type: "button", style: { ...styles.buttonBase, ...styles.save, opacity: blocked ? 0.4 : 1 }, disabled: blocked, onClick: props.save, children: state.saving ? "Saving…" : "Save" })
                ]
              })
            ]
          }) : null
        ]
      });
    }
    //#endregion

    //#region apply
    var inject = ["slots", "settingsScope"];
    function apply(ctx) {
      var scope = ctx.settingsScope.bind({ namespace: NS });
      var controller = new TokenloomCardController(scope);
      ctx.effect(() => ctx.slots.inject("settings.plugin.item", function*() {
        yield ctx.slots.register({
          name: "settings.plugin.item",
          key: NS,
          inject: function() { return controller.inject(); }
        }, TokenloomCard);
      }), "web-search-tokenloom: settings card");
    }
    //#endregion

    exports.apply = apply;
    exports.inject = inject;
    return module.exports;
  }
});
