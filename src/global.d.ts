declare global {
    interface Window {
        /** Set by `open_tab_in_new_window` init script. Consumed once on mount, then deleted. */
        __rssh_clone?: string;
    }
}

export {};
