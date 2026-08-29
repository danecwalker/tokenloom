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

window.__ModuleLoader__.load({
	id: "@dane/dsh-web-search-tokenloom",
	factory: (require) => {
		var react = require("react");
		var useSyncExternalStore = react.useSyncExternalStore;
		var react_jsx_runtime = require("react/jsx-runtime");
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

		//#region card component
		var styles = {
			card: { border: "1px solid var(--dsh-border, #3f3f46)", borderRadius: "10px", padding: "16px", marginBottom: "12px" },
			title: { fontSize: "15px", fontWeight: 600, margin: "0 0 2px" },
			description: { fontSize: "13px", opacity: 0.65, margin: "0 0 12px" },
			readOnly: { fontSize: "12px", opacity: 0.6, margin: "0 0 8px" },
			label: { display: "block", fontSize: "13px", fontWeight: 500, margin: "10px 0 4px" },
			hint: { display: "block", fontSize: "12px", opacity: 0.55, margin: "3px 0 0" },
			input: { width: "100%", boxSizing: "border-box", padding: "7px 10px", borderRadius: "7px", border: "1px solid var(--dsh-border, #3f3f46)", background: "transparent", color: "inherit", fontSize: "13px" },
			badge: { display: "inline-block", fontSize: "11px", borderRadius: "999px", padding: "1px 8px", marginLeft: "8px", border: "1px solid var(--dsh-border, #3f3f46)", opacity: 0.75 },
			invalid: { color: "#ef4444", fontSize: "12px", margin: "3px 0 0" },
			footer: { display: "flex", alignItems: "center", gap: "8px", marginTop: "14px" },
			button: { padding: "6px 14px", borderRadius: "7px", border: "1px solid var(--dsh-border, #3f3f46)", background: "transparent", color: "inherit", fontSize: "13px", cursor: "pointer" },
			note: { fontSize: "12px", opacity: 0.6 }
		};
		function FieldRow(props) {
			return jsxRuntime.jsxs("div", {
				children: [
					jsxRuntime.jsxs("label", { style: styles.label, htmlFor: props.id, children: [
						props.label,
						props.overridden ? jsxRuntime.jsx("span", { style: styles.badge, children: "Overridden" }) : null
					] }),
					jsxRuntime.jsx("input", {
						id: props.id,
						style: styles.input,
						type: "text",
						inputMode: props.numeric ? "numeric" : undefined,
						disabled: props.disabled,
						value: props.text,
						onChange: function(event) { props.onEdit(event.target.value); }
					}),
					props.hint ? jsxRuntime.jsx("span", { style: styles.hint, children: props.hint }) : null,
					props.invalid ? jsxRuntime.jsx("span", { style: styles.invalid, children: props.invalidLabel }) : null,
					props.overridden ? jsxRuntime.jsx("button", { style: { ...styles.button, marginTop: "4px", fontSize: "12px", opacity: 0.8 }, type: "button", disabled: props.disabled, onClick: props.onReset, children: "Reset to default" }) : null
				]
			});
		}
		function TokenloomCard(props) {
			// The slot framework converts `hooks` entries into `useX` selectors;
			// fall back to the raw store if it passes them through verbatim.
			var store = props.useTokenloomCard ?? props.hooks.tokenloomCard;
			var state = useSyncExternalStore(store.subscribe, store.getSnapshot);
			var disabled = !state.writable || state.saving;
			var fields = [
				{ field: "bin", id: "plugin-config-tokenloom-bin", label: "Binary", hint: "Path or $PATH name of the tokenloom CLI. Leave blank to use the default.", numeric: false },
				{ field: "maxResults", id: "plugin-config-tokenloom-max-results", label: "Max results per search", hint: "How many federated results one search returns.", numeric: true },
				{ field: "timeoutMs", id: "plugin-config-tokenloom-search-timeout", label: "Search timeout (ms)", hint: "Wall-clock budget for one search invocation.", numeric: true },
				{ field: "fetchTimeoutMs", id: "plugin-config-tokenloom-fetch-timeout", label: "Fetch timeout (ms)", hint: "Budget for one fetch; the SPA fallback ladder can run long.", numeric: true }
			];
			return jsxRuntime.jsxs("div", { style: styles.card, children: [
				jsxRuntime.jsx("h3", { style: styles.title, children: "tokenloom" }),
				jsxRuntime.jsx("p", { style: styles.description, children: "Federated web search and 7-layer-sanitised page fetching through the tokenloom CLI." }),
				!state.writable ? jsxRuntime.jsx("p", { style: styles.readOnly, children: "This deployment stores settings read-only." }) : null,
				fields.map(function(f) {
					return jsxRuntime.jsx(FieldRow, {
						id: f.id,
						label: f.label,
						hint: f.hint,
						numeric: f.numeric,
						disabled: disabled,
						overriddenLabel: "Overridden",
						invalidLabel: "Enter a number, or leave blank to use the default.",
						...state[f.field],
						onEdit: function(text) { props.edit(f.field, text); },
						onReset: function() { props.resetField(f.field); }
					}, f.field);
				}),
				jsxRuntime.jsxs("div", { style: styles.footer, children: [
					jsxRuntime.jsx("button", { style: styles.button, type: "button", disabled: disabled || !state.dirty || state.invalid, onClick: props.save, children: state.saving ? "Saving…" : "Save" }),
					jsxRuntime.jsx("button", { style: { ...styles.button, opacity: 0.8 }, type: "button", disabled: disabled || !state.dirty, onClick: props.discard, children: "Discard" }),
					state.failed ? jsxRuntime.jsx("span", { style: styles.note, children: "The deployment did not accept these values; they were left for you to correct." }) : null,
					state.dirty && !state.failed ? jsxRuntime.jsx("span", { style: styles.note, children: "Unsaved" }) : null
				] })
			] });
		}
		//#endregion

		//#region apply
		var inject = ["slots", "settingsScope"];
		function apply(ctx) {
			var scope = ctx.settingsScope.bind({ namespace: NS });
			var controller = new TokenloomCardController(scope);
			ctx.effect(() => ctx.slots.inject("settings.plugin.item", function* () {
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
