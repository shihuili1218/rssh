declare global {
    interface Window {
        /** Set by `open_tab_in_new_window` init script. Consumed once on mount, then deleted. */
        __rssh_clone?: string;
        /** Set by `analyze_locally` tool: opens local shell + auto-starts AI session and sends task. */
        __rssh_ai_handoff?: string;
    }
}

export {};
