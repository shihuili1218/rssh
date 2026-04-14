<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import type { Group } from "../stores/app.svelte.ts";
  import * as app from "../stores/app.svelte.ts";

  let groups = $state<Group[]>([]);
  let adding = $state(false);
  let editId = $state<string | null>(null);

  // Form fields
  let formName = $state("");
  let formColor = $state("#4A6CF7");
  let formOrder = $state(0);

  let deleting = $state<string | null>(null);

  onMount(refresh);

  async function refresh() {
    groups = await app.loadGroups();
  }

  function startAdd() {
    adding = true;
    editId = null;
    formName = "";
    formColor = "#4A6CF7";
    formOrder = 0;
  }

  function startEdit(g: Group) {
    adding = false;
    editId = g.id;
    formName = g.name;
    formColor = g.color;
    formOrder = g.sort_order;
  }

  function cancelForm() {
    adding = false;
    editId = null;
  }

  async function saveNew() {
    if (!formName.trim()) return;
    const group: Group = {
      id: crypto.randomUUID(),
      name: formName.trim(),
      color: formColor,
      sort_order: formOrder,
    };
    try {
      await invoke("create_group", { group });
      adding = false;
      await refresh();
    } catch (e: any) { alert(String(e)); }
  }

  async function saveEdit() {
    if (!editId || !formName.trim()) return;
    const group: Group = {
      id: editId,
      name: formName.trim(),
      color: formColor,
      sort_order: formOrder,
    };
    try {
      await invoke("update_group", { group });
      editId = null;
      await refresh();
    } catch (e: any) { alert(String(e)); }
  }

  async function remove(id: string) {
    deleting = id;
    try {
      await invoke("delete_group", { id });
      if (editId === id) editId = null;
      await refresh();
    } catch (e: any) { alert("Delete failed: " + String(e)); }
    finally { deleting = null; }
  }
</script>

<div class="page">
  <div class="toolbar">
    <button class="btn btn-accent btn-sm" onclick={startAdd}>+ New Group</button>
  </div>

  {#if adding}
    <div class="card inline-form">
      <label>Name</label>
      <input type="text" bind:value={formName} placeholder="Group name" />
      <label>Color</label>
      <input type="color" bind:value={formColor} />
      <label>Sort Order</label>
      <input type="number" bind:value={formOrder} min="0" />
      <div class="form-actions">
        <button class="btn btn-accent btn-sm" onclick={saveNew} disabled={!formName.trim()}>Save</button>
        <button class="btn btn-sm" onclick={cancelForm}>Cancel</button>
      </div>
    </div>
  {/if}

  {#each groups as g (g.id)}
    {#if editId === g.id}
      <div class="card inline-form">
        <label>Name</label>
        <input type="text" bind:value={formName} placeholder="Group name" />
        <label>Color</label>
        <input type="color" bind:value={formColor} />
        <label>Sort Order</label>
        <input type="number" bind:value={formOrder} min="0" />
        <div class="form-actions">
          <button class="btn btn-accent btn-sm" onclick={saveEdit} disabled={!formName.trim()}>Save</button>
          <button class="btn btn-sm" onclick={cancelForm}>Cancel</button>
        </div>
      </div>
    {:else}
      <div class="card item-row">
        <div class="item-info">
          <span class="color-swatch" style="background: {g.color}"></span>
          <div>
            <div class="item-name">{g.name}</div>
            <div class="item-sub">order: {g.sort_order}</div>
          </div>
        </div>
        <div class="item-actions">
          <button class="btn btn-sm" onclick={() => startEdit(g)}>Edit</button>
          <button class="btn btn-sm btn-danger" onclick={() => remove(g.id)} disabled={deleting === g.id}>
            {deleting === g.id ? "..." : "Delete"}
          </button>
        </div>
      </div>
    {/if}
  {:else}
    {#if !adding}
      <p class="empty">No groups yet</p>
    {/if}
  {/each}
</div>

<style>
  .page { padding: 24px; }
  .toolbar { display: flex; justify-content: flex-end; margin-bottom: 16px; }
  .item-row { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .item-info { display: flex; align-items: center; gap: 10px; }
  .item-name { font-weight: 600; font-size: 14px; }
  .item-sub { font-size: 12px; color: var(--text-sub); }
  .item-actions { display: flex; gap: 6px; }
  .color-swatch {
    width: 20px; height: 20px; border-radius: 4px; flex-shrink: 0;
    border: 1px solid var(--divider);
  }
  .inline-form {
    display: flex; flex-direction: column; gap: 8px;
    padding: 14px; margin-bottom: 10px;
  }
  .inline-form input[type="color"] { width: 48px; height: 32px; padding: 2px; border: 1px solid var(--divider); border-radius: 4px; cursor: pointer; }
  .inline-form input[type="number"] { width: 80px; }
  .form-actions { display: flex; gap: 6px; margin-top: 4px; }
  .empty { text-align: center; color: var(--text-dim); padding: 32px; }
</style>
