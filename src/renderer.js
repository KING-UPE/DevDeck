const invoke = window.__TAURI__.core.invoke;
const { listen } = window.__TAURI__.event;
// === CUSTOM UI MODALS ===
function customAlert(message) {
    return new Promise(resolve => {
        const modal = document.getElementById('custom-alert-modal');
        const msgEl = document.getElementById('custom-alert-message');
        const okBtn = document.getElementById('custom-alert-ok-btn');
        msgEl.textContent = message;
        modal.style.display = 'flex';
        
        const cleanup = () => {
            modal.style.display = 'none';
            okBtn.removeEventListener('click', onClick);
            resolve();
        };
        
        const onClick = () => cleanup();
        okBtn.addEventListener('click', onClick);
    });
}

function customPrompt(message, defaultText = '') {
    return new Promise(resolve => {
        const modal = document.getElementById('custom-prompt-modal');
        const msgEl = document.getElementById('custom-prompt-message');
        const inputEl = document.getElementById('custom-prompt-input');
        const okBtn = document.getElementById('custom-prompt-ok-btn');
        const cancelBtn = document.getElementById('custom-prompt-cancel-btn');
        
        msgEl.textContent = message;
        inputEl.value = defaultText;
        modal.style.display = 'flex';
        inputEl.focus();
        inputEl.select();
        
        const cleanup = (result) => {
            modal.style.display = 'none';
            okBtn.removeEventListener('click', onOk);
            cancelBtn.removeEventListener('click', onCancel);
            inputEl.removeEventListener('keydown', onKeyDown);
            resolve(result);
        };
        
        const onOk = () => cleanup(inputEl.value);
        const onCancel = () => cleanup(null);
        const onKeyDown = (e) => {
            if (e.key === 'Enter') onOk();
            if (e.key === 'Escape') onCancel();
        };
        
        okBtn.addEventListener('click', onOk);
        cancelBtn.addEventListener('click', onCancel);
        inputEl.addEventListener('keydown', onKeyDown);
    });
}

function isSubPath(parentPath, childPath) {
    if (!parentPath || !childPath) return false;
    const p = parentPath.replace(/\\/g, '/').replace(/\/$/, '').toLowerCase() + '/';
    const c = childPath.replace(/\\/g, '/').replace(/\/$/, '').toLowerCase() + '/';
    return c.startsWith(p);
}

// Helper for case-insensitive array path matching
function isPathInArray(arr, path) {
    if (!arr || !path) return false;
    const normalized = path.replace(/\\/g, '/').toLowerCase();
    return arr.some(item => item.replace(/\\/g, '/').toLowerCase() === normalized);
}

// === PERSISTENT STATE (SQLite) ===
// These used to live in localStorage, which the Rust side cannot read - so the
// phone never saw workspace names, pins or custom project names. They are rows
// in the database now; `stateCache` holds the last loaded copy so reads stay
// synchronous the way the surrounding code expects.
let stateCache = {};

function stored(key, fallback) {
    return stateCache[key] !== undefined ? stateCache[key] : fallback;
}

/// Flags are written as 'true' by the UI but imported as '1'; accept both.
function storedFlag(key) {
    const v = stored(key, '');
    return v === '1' || v === 'true';
}

function persist(key, value) {
    const text = typeof value === 'string' ? value : JSON.stringify(value);
    stateCache[key] = text;
    invoke('db_save_state', { key: key, value: text })
        .catch(e => console.error('Could not save ' + key, e));
}

let workspaces = [];
let allProjects = [];
let hiddenProjects = [];
let knownProjects = [];
let customProjectNames = {};
let pinnedProjects = [];
let defaultIde = 'code';

/// Hand any remaining localStorage contents to the database once, then load
/// everything back out of it.
async function bootstrapState() {
    try {
        if (!(await invoke('db_is_migrated'))) {
            const legacy = {
                workspaces: JSON.parse(localStorage.getItem('workspaces') || '[]'),
                known_projects: JSON.parse(localStorage.getItem('knownProjects') || '[]'),
                hidden_projects: JSON.parse(localStorage.getItem('hiddenProjects') || '[]'),
                pinned_projects: JSON.parse(localStorage.getItem('pinnedProjects') || '[]'),
                custom_project_names: JSON.parse(localStorage.getItem('customProjectNames') || '{}'),
                default_ide: localStorage.getItem('defaultIde'),
                tour_completed: localStorage.getItem('tourCompleted') === 'true'
            };
            const moved = await invoke('db_import_legacy', { legacy: legacy });
            if (moved > 0) console.info('Moved ' + moved + ' records into the database.');
        }
        stateCache = await invoke('db_load_state');
    } catch (e) {
        // A failure here must not blank the UI; carry on with defaults.
        console.error('Could not load state from the database', e);
    }

    workspaces = JSON.parse(stored('workspaces', '[]'));
    hiddenProjects = JSON.parse(stored('hiddenProjects', '[]'));
    knownProjects = JSON.parse(stored('knownProjects', '[]'));
    customProjectNames = JSON.parse(stored('customProjectNames', '{}'));
    pinnedProjects = JSON.parse(stored('pinnedProjects', '[]'));
    defaultIde = stored('defaultIde', 'code');
}

const IDE_TOOLS = [
    // GUI IDEs & Editors
    { id: 'code', name: 'VS Code', badge: 'Default', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="#007ACC"><path d="M23.15 2.587L18.21.21a1.494 1.494 0 0 0-1.705.29l-9.46 8.63L2.83 5.75a.997.997 0 0 0-1.409.096l-1.18 1.34a1 1 0 0 0 .096 1.409l4.51 3.96-4.51 3.96a1 1 0 0 0-.096 1.409l1.18 1.34a.998.998 0 0 0 1.409.096l4.215-3.38 9.46 8.63a1.494 1.494 0 0 0 1.705.29l4.94-2.377A1.5 1.5 0 0 0 24 21.75V3.75a1.5 1.5 0 0 0-.85-1.163zM18 17.55l-7.25-5.55L18 6.45v11.1z"/></svg>` },
    { id: 'code-insiders', name: 'VS Code Insiders', badge: 'Insiders', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="#23A566"><path d="M23.15 2.587L18.21.21a1.494 1.494 0 0 0-1.705.29l-9.46 8.63L2.83 5.75a.997.997 0 0 0-1.409.096l-1.18 1.34a1 1 0 0 0 .096 1.409l4.51 3.96-4.51 3.96a1 1 0 0 0-.096 1.409l1.18 1.34a.998.998 0 0 0 1.409.096l4.215-3.38 9.46 8.63a1.494 1.494 0 0 0 1.705.29l4.94-2.377A1.5 1.5 0 0 0 24 21.75V3.75a1.5 1.5 0 0 0-.85-1.163zM18 17.55l-7.25-5.55L18 6.45v11.1z"/></svg>` },
    { id: 'cursor', name: 'Cursor', badge: 'AI IDE', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M12 2L2 7v10l10 5 10-5V7L12 2zm8 14.26l-8 4-8-4V8.74l8-4 8 4v7.52z" fill="#00D2FF"/><path d="M12 6.5L6 9.5v5l6 3 6-3v-5l-6-3z" fill="#00D2FF"/></svg>` },
    { id: 'windsurf', name: 'Windsurf', badge: 'AI IDE', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M4 18c3-4 6-6 10-6s7 2 7 6H4zm0-6c2-4 5-7 10-7s8 3 9 7H4z" fill="#00E5FF"/></svg>` },
    { id: 'idea', name: 'IntelliJ IDEA', badge: 'JetBrains', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><rect width="24" height="24" rx="4" fill="url(#ij_g)"/><path d="M4 18h8v2H4v-2zm0-12h3v10H4V6zm5 0h3v7h-3V6zm-5 5h7v2H4v-2z" fill="#FFF"/><defs><linearGradient id="ij_g" x1="0" y1="0" x2="24" y2="24"><stop offset="0%" stop-color="#FE2857"/><stop offset="100%" stop-color="#087CFA"/></linearGradient></defs></svg>` },
    { id: 'webstorm', name: 'WebStorm', badge: 'JetBrains', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><rect width="24" height="24" rx="4" fill="url(#ws_g)"/><text x="2.5" y="16.5" font-family="sans-serif" font-weight="900" font-size="11" fill="#FFF">WS</text><defs><linearGradient id="ws_g" x1="0" y1="0" x2="24" y2="24"><stop offset="0%" stop-color="#087CFA"/><stop offset="100%" stop-color="#30D5C8"/></linearGradient></defs></svg>` },
    { id: 'pycharm', name: 'PyCharm', badge: 'JetBrains', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><rect width="24" height="24" rx="4" fill="url(#pc_g)"/><text x="3" y="16.5" font-family="sans-serif" font-weight="900" font-size="11" fill="#FFF">PC</text><defs><linearGradient id="pc_g" x1="0" y1="0" x2="24" y2="24"><stop offset="0%" stop-color="#21D789"/><stop offset="100%" stop-color="#087CFA"/></linearGradient></defs></svg>` },
    { id: 'sublime', name: 'Sublime Text', badge: 'Editor', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M3 16.5L14 21l7-3-11-4.5L3 16.5zM21 7.5L10 3 3 6l11 4.5L21 7.5zM3 11.5L14 16l7-3-11-4.5L3 11.5z" fill="#FF9800"/></svg>` },
    { id: 'fleet', name: 'JetBrains Fleet', badge: 'JetBrains', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><rect width="24" height="24" rx="4" fill="url(#fl_g)"/><text x="3.5" y="16.5" font-family="sans-serif" font-weight="900" font-size="11" fill="#FFF">FL</text><defs><linearGradient id="fl_g" x1="0" y1="0" x2="24" y2="24"><stop offset="0%" stop-color="#7B2CBF"/><stop offset="100%" stop-color="#E0AAFF"/></linearGradient></defs></svg>` },
    { id: 'studio', name: 'Android Studio', badge: 'Android', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><circle cx="12" cy="12" r="10" fill="#3DDC84"/><path d="M7 14l2.5-4.5M17 14l-2.5-4.5M9 7a1 1 0 0 1 2 0v1h2V7a1 1 0 1 1 2 0v1h1a2 2 0 0 1 2 2v4H6v-4a2 2 0 0 1 2-2h1V7z" fill="#1A1D2D"/></svg>` },
    { id: 'zed', name: 'Zed Editor', badge: 'Fast IDE', category: 'IDE', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><rect width="24" height="24" rx="4" fill="#18181B"/><path d="M6 7h12l-9 10h9" stroke="#A1A1AA" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/></svg>` },
    
    // AI Agents & CLI Tools
    { id: 'antigravity', name: 'Antigravity AI', badge: 'AI Agent', category: 'AI', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M12 2L3 15h6v7l9-13h-6V2z" fill="url(#agy_g)"/><defs><linearGradient id="agy_g" x1="0" y1="0" x2="24" y2="24"><stop offset="0%" stop-color="#BB86FC"/><stop offset="100%" stop-color="#03DAC6"/></linearGradient></defs></svg>` },
    { id: 'claude', name: 'Claude Code CLI', badge: 'AI Agent', category: 'AI', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M12 2L14.5 9.5L22 12L14.5 14.5L12 22L9.5 14.5L2 12L9.5 9.5L12 2z" fill="#D97757"/></svg>` },
    { id: 'aider', name: 'Aider AI Agent', badge: 'AI Agent', category: 'AI', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><rect width="24" height="24" rx="5" fill="#8B5CF6"/><path d="M7 17L12 7l5 10m-8.5-3h7" stroke="#FFF" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>` },
    { id: 'copilot', name: 'GitHub Copilot CLI', badge: 'AI Agent', category: 'AI', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M12 2a10 10 0 00-10 10c0 4.42 2.87 8.17 6.84 9.5.5.08.66-.23.66-.5v-1.69c-2.77.6-3.36-1.34-3.36-1.34-.46-1.16-1.11-1.47-1.11-1.47-.91-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.87 1.52 2.34 1.07 2.91.83.1-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.92 0-1.11.38-2 1.03-2.71-.1-.25-.45-1.29.1-2.64 0 0 .84-.27 2.75 1.02.79-.22 1.65-.33 2.5-.33.85 0 1.71.11 2.5.33 1.91-1.29 2.75-1.02 2.75-1.02.55 1.35.2 2.39.1 2.64.65.71 1.03 1.6 1.03 2.71 0 3.82-2.34 4.66-4.57 4.91.36.31.69.92.69 1.85V21c0 .27.16.59.67.5C19.14 20.16 22 16.42 22 12A10 10 0 0012 2z" fill="#6E40C9"/></svg>` },
    { id: 'gemini', name: 'Gemini CLI', badge: 'AI Agent', category: 'AI', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M12 2C12 7.52 7.52 12 2 12c5.52 0 10 4.48 10 10 0-5.52 4.48-10 10-10-5.52 0-10-4.48-10-10z" fill="url(#gem_g)"/><defs><linearGradient id="gem_g" x1="0" y1="0" x2="24" y2="24"><stop offset="0%" stop-color="#1A73E8"/><stop offset="50%" stop-color="#8AB4F8"/><stop offset="100%" stop-color="#E8EAED"/></linearGradient></defs></svg>` },

    // System Utilities
    { id: 'explorer', name: 'File Explorer', badge: 'System', category: 'System', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" fill="#FFA726"/></svg>` },
    { id: 'terminal', name: 'Native Terminal', badge: 'CLI', category: 'System', icon: `<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#7AA2F7" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"></polyline><line x1="12" y1="19" x2="20" y2="19"></line></svg>` }
];

function getIdeInfo(id) {
    return IDE_TOOLS.find(t => t.id === id) || IDE_TOOLS[0];
}

let showHidden = false;
let activeWorkspace = null;
let activeProject = null;
let runningProcesses = new Set();
let processLogs = {}; // { processKey: [logs...] }
let processUrls = {}; // { processKey: Set<String> }

// === ONBOARDING TOUR LOGIC ===
const tourSteps = [
    { el: '#add-workspace-btn', title: 'Add Workspaces', desc: 'A workspace is simply a folder that contains your projects. Click here to manually add a folder, or auto-detect later.' },
    { el: '.workspace-item', title: 'Select Workspace', desc: 'Your workspaces will appear here. Selecting one will instantly list all of its contained projects.' },
    { el: '.projects-footer', title: 'Search & Filters', desc: 'Use the bottom bar to search, filter by framework, and unhide projects you have deselected.' },
    { el: '.project-details', title: 'Run & Manage Scripts', desc: 'Once a project is selected, all its package.json and Cargo scripts appear here. You can run, stop, or execute custom commands directly!' },
    { el: '#process-manager-btn', title: 'Process Manager', desc: 'Never worry about ghost processes! Kill lingering Node, Rust, or Python servers easily with one click.' }
];

let currentTourStep = 0;
const tourWelcomeModal = document.getElementById('tour-welcome-modal');
const tourMask = document.getElementById('tour-mask');
const tourHighlightBox = document.getElementById('tour-highlight-box');
const tourTooltip = document.getElementById('tour-tooltip');
const tourTitle = document.getElementById('tour-title');
const tourDesc = document.getElementById('tour-desc');
const tourStepIndicator = document.getElementById('tour-step-indicator');


// Deferred: whether the tour has run is only known once state has loaded.
function maybeShowTour() {
    if (!storedFlag('tourCompleted')) {
        tourWelcomeModal.style.display = 'flex';
    }
}

document.getElementById('skip-tour-btn').addEventListener('click', () => {
    persist('tourCompleted', 'true');
    tourWelcomeModal.style.display = 'none';
});

document.getElementById('start-tour-btn').addEventListener('click', () => {
    tourWelcomeModal.style.display = 'none';
    startTour();
});

const infoMenuBtn = document.getElementById('info-menu-btn');
const infoDropdown = document.getElementById('info-dropdown');
const openDocsBtn = document.getElementById('open-docs-btn');
const restartTourBtnNew = document.getElementById('restart-tour-btn-new');

if (infoMenuBtn && infoDropdown) {
    infoMenuBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        infoDropdown.style.display = infoDropdown.style.display === 'none' ? 'block' : 'none';
    });

    document.addEventListener('click', () => {
        infoDropdown.style.display = 'none';
    });

    if (openDocsBtn) {
        openDocsBtn.addEventListener('click', () => {
            window.handleDirectLinkClick('https://king-upe.github.io/DevDeck/help.html');
            infoDropdown.style.display = 'none';
        });
    }

    if (restartTourBtnNew) {
        restartTourBtnNew.addEventListener('click', () => {
            persist('tourCompleted', '');
            tourWelcomeModal.style.display = 'flex';
            infoDropdown.style.display = 'none';
        });
    }
}

function startTour() {
    currentTourStep = 0;
    tourMask.style.display = 'block';
    tourHighlightBox.style.display = 'block';
    tourTooltip.style.display = 'block';
    
    // Inject Dummy Data for Tour
    secondarySidebar.classList.remove('collapsed');
    
    // Ensure workspace list has at least one item visually
    if (!workspaceListEl.innerHTML.includes('workspace-item')) {
        workspaceListEl.innerHTML = `<div class="workspace-item active" style="padding: 0.75rem; border-radius: 8px; margin-bottom: 0.25rem; background: var(--accent); color: var(--bg-color);">
            <div style="font-weight: 600; font-size: 0.9rem;">Example Workspace</div>
            <div style="font-size: 0.7rem; opacity: 0.8; margin-top: 2px;">C:\\Projects</div>
        </div>`;
    }
    
    // Ensure project list has at least one item visually
    projectListEl.innerHTML = `<div class="project-item active" style="padding: 0.75rem; border-radius: 8px; margin-bottom: 0.5rem; background: var(--surface-light); border: 1px solid var(--border); border-left: 3px solid var(--accent);">
        <div style="font-weight: 600; font-size: 0.95rem; color: var(--accent);">my-awesome-app</div>
        <div style="font-size: 0.75rem; color: var(--text-muted); margin-top: 4px;">React</div>
    </div>`;
    
    // Ensure scripts section is visible with dummy data
    activeProjectHeader.style.display = 'block';
    activeProjectName.textContent = 'my-awesome-app';
    activeProjectPath.textContent = 'C:\\Projects\\my-awesome-app';
    scriptsSection.style.display = 'block';
    scriptsGrid.innerHTML = `
        <div class="script-btn-container">
            <div style="font-weight: 600; font-size: 0.85rem; color: var(--text-primary); margin-bottom: 0.25rem;">dev</div>
            <div style="display: flex; gap: 0.5rem; flex-wrap: wrap;">
                <button class="btn btn-primary script-btn" style="flex: 1; padding: 0.25rem 0.5rem;"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: 4px; vertical-align: middle;"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg> Run</button>
            </div>
        </div>
    `;
    
    showTourStep(0);
}

function showTourStep(index) {
    if (index >= tourSteps.length) {
        endTour();
        return;
    }
    const step = tourSteps[index];
    const targetEl = document.querySelector(step.el);
    if (!targetEl) return;
    
    // Smooth scroll if necessary
    targetEl.scrollIntoView({ behavior: 'smooth', block: 'center' });
    
    const rect = targetEl.getBoundingClientRect();
    
    // Move highlight box
    tourHighlightBox.style.top = (rect.top - 5) + 'px';
    tourHighlightBox.style.left = (rect.left - 5) + 'px';
    tourHighlightBox.style.width = (rect.width + 10) + 'px';
    tourHighlightBox.style.height = (rect.height + 10) + 'px';
    
    // Position tooltip
    tourTooltip.style.top = (rect.bottom + 20) + 'px';
    tourTooltip.style.left = (rect.left) + 'px';
    
    // Prevent tooltip going off screen
    if (rect.bottom + 200 > window.innerHeight) {
        tourTooltip.style.top = (Math.max(10, rect.top - tourTooltip.offsetHeight - 20)) + 'px'; // Show above
    }
    if (rect.left + 320 > window.innerWidth) {
        tourTooltip.style.left = (window.innerWidth - 340) + 'px'; // Shift left
    }
    
    tourTitle.textContent = step.title;
    tourDesc.textContent = step.desc;
    tourStepIndicator.textContent = `${index + 1}/${tourSteps.length}`;
}

document.getElementById('tour-next-btn').addEventListener('click', () => {
    currentTourStep++;
    showTourStep(currentTourStep);
});

document.getElementById('tour-prev-btn').addEventListener('click', () => {
    if (currentTourStep > 0) {
        currentTourStep--;
        showTourStep(currentTourStep);
    }
});

function endTour() {
    tourMask.style.display = 'none';
    tourHighlightBox.style.display = 'none';
    tourTooltip.style.display = 'none';
    
    // Clean up Dummy Data
    secondarySidebar.classList.add('collapsed');
    activeProjectHeader.style.display = 'none';
    scriptsSection.style.display = 'none';
    scriptsGrid.innerHTML = '';
    renderWorkspaces();
    renderProjects();
    
    persist('tourCompleted', 'true');
}


// DOM Elements
const addWorkspaceBtn = document.getElementById('add-workspace-btn');
const scanAllBtn = document.getElementById('scan-all-btn');
const scanProgressEl = document.getElementById('scan-progress');
const workspaceListEl = document.getElementById('workspace-list');
const scanBtn = document.getElementById('scan-btn');
const projectListEl = document.getElementById('project-list');
const activeProjectHeader = document.getElementById('active-project-header');
const activeProjectName = document.getElementById('active-project-name');
const activeProjectPath = document.getElementById('active-project-path');
const scriptsSection = document.getElementById('scripts-section');
const scriptsGrid = document.getElementById('scripts-grid');
const terminalTabs = document.getElementById('terminal-tabs');
const terminalBody = document.getElementById('terminal-body');
const customCmdInput = document.getElementById('custom-cmd-input');
const runCustomCmdBtn = document.getElementById('run-custom-cmd-btn');

const manageHiddenBtn = document.getElementById('manage-hidden-btn');
const projectSearch = document.getElementById('project-search');
const projectTypeFilter = document.getElementById('project-type-filter');
const renameProjectBtn = document.getElementById('rename-project-btn');
const openExternalTermBtn = document.getElementById('open-external-term-btn');
const stdinInput = document.getElementById('stdin-input');

const ideLauncherContainer = document.getElementById('ide-launcher-container');
const openIdeBtn = document.getElementById('open-ide-btn');
const openIdeIcon = document.getElementById('open-ide-icon');
const openIdeLabel = document.getElementById('open-ide-label');
const openIdeDropdownToggle = document.getElementById('open-ide-dropdown-toggle');
const ideDropdownMenu = document.getElementById('ide-dropdown-menu');

function updateMainIdeButton() {
    const current = getIdeInfo(defaultIde);
    if (openIdeIcon) openIdeIcon.innerHTML = current.icon;
    if (openIdeLabel) openIdeLabel.textContent = current.name;
    if (openIdeBtn) openIdeBtn.title = `Open project in ${current.name}`;
}

function renderIdeDropdown() {
    if (!ideDropdownMenu) return;
    ideDropdownMenu.innerHTML = '';

    const categories = [
        { key: 'IDE', label: 'IDEs & Editors' },
        { key: 'AI', label: 'AI Agents & CLI' },
        { key: 'System', label: 'System Utilities' }
    ];

    categories.forEach(cat => {
        const toolsInCat = IDE_TOOLS.filter(t => t.category === cat.key);
        if (toolsInCat.length === 0) return;

        const header = document.createElement('div');
        header.style.fontSize = '0.68rem';
        header.style.fontWeight = '700';
        header.style.textTransform = 'uppercase';
        header.style.letterSpacing = '0.05em';
        header.style.color = 'var(--text-secondary)';
        header.style.padding = '0.4rem 0.6rem 0.2rem 0.6rem';
        header.style.marginTop = '0.2rem';
        header.textContent = cat.label;
        ideDropdownMenu.appendChild(header);

        toolsInCat.forEach(tool => {
            const option = document.createElement('div');
            option.className = `ide-option ${tool.id === defaultIde ? 'is-default' : ''}`;
            
            option.innerHTML = `
                <div class="ide-option-info">
                    <span class="ide-option-icon">${tool.icon}</span>
                    <span>${tool.name}</span>
                </div>
                <span class="ide-option-badge">${tool.id === defaultIde ? 'Default' : tool.badge}</span>
            `;

            option.addEventListener('click', async (e) => {
                e.stopPropagation();
                ideDropdownMenu.style.display = 'none';
                if (activeProject) {
                    try {
                        await window.__TAURI__.core.invoke('open_in_editor', { path: activeProject.path, tool: tool.id });
                    } catch (err) {
                        await customAlert(`Failed to launch ${tool.name}: ` + err);
                    }
                }
            });

            option.addEventListener('contextmenu', (e) => {
                e.preventDefault();
                defaultIde = tool.id;
                persist('defaultIde', defaultIde);
                updateMainIdeButton();
                renderIdeDropdown();
                renderProjects();
            });

            ideDropdownMenu.appendChild(option);
        });
    });
}

if (openIdeBtn) {
    openIdeBtn.addEventListener('click', async () => {
        if (!activeProject) return;
        try {
            await window.__TAURI__.core.invoke('open_in_editor', { path: activeProject.path, tool: defaultIde });
        } catch (err) {
            await customAlert(`Could not launch ${getIdeInfo(defaultIde).name}: ` + err);
        }
    });
}

if (openIdeDropdownToggle) {
    openIdeDropdownToggle.addEventListener('click', (e) => {
        e.stopPropagation();
        const isVisible = ideDropdownMenu.style.display === 'flex' || ideDropdownMenu.style.display === 'block';
        ideDropdownMenu.style.display = isVisible ? 'none' : 'flex';
        if (!isVisible) renderIdeDropdown();
    });
}

document.addEventListener('click', (e) => {
    if (ideDropdownMenu && ideLauncherContainer && !ideLauncherContainer.contains(e.target)) {
        ideDropdownMenu.style.display = 'none';
    }
});


stdinInput.addEventListener('keydown', async (e) => {
    if (e.key === 'Enter') {
        const input = stdinInput.value;
        if (input.trim() !== '' && activeTerminalTab) {
            try {
                await window.__TAURI__.core.invoke('write_to_stdin', { 
                    processKey: activeTerminalTab, 
                    input: input 
                });
                stdinInput.value = '';
            } catch (err) {
                await customAlert("Failed to send input: " + err);
            }
        }
    }
});

const scanSelectionModal = document.getElementById('scan-selection-modal');
const scanSelectionList = document.getElementById('scan-selection-list');
const scanSelectionCancelBtn = document.getElementById('scan-selection-cancel-btn');
const scanSelectionSaveBtn = document.getElementById('scan-selection-save-btn');
const scanSelectAllBtn = document.getElementById('scan-select-all-btn');
const scanUnselectAllBtn = document.getElementById('scan-unselect-all-btn');

let tempScannedProjects = [];
let tempWorkspacePath = null;

const hiddenProjectsModal = document.getElementById('hidden-projects-modal');
const closeHiddenModalBtn = document.getElementById('close-hidden-modal');
const hiddenProjectSearch = document.getElementById('hidden-project-search');
const hiddenProjectsList = document.getElementById('hidden-projects-list');

// Delete Workspace Modal
const deleteWorkspaceModal = document.getElementById('delete-workspace-modal');
const cancelDeleteWorkspaceBtn = document.getElementById('cancel-delete-workspace-btn');
const confirmDeleteWorkspaceBtn = document.getElementById('confirm-delete-workspace-btn');
let workspaceToDelete = null;

cancelDeleteWorkspaceBtn.addEventListener('click', () => {
    deleteWorkspaceModal.style.display = 'none';
    workspaceToDelete = null;
});

confirmDeleteWorkspaceBtn.addEventListener('click', () => {
    if (workspaceToDelete) {
        workspaces = workspaces.filter(w => (w.path || w) !== workspaceToDelete);
        
        // We wipe knownProjects, hiddenProjects, and customProjectNames so if they re-add the workspace,
        // it acts as a fresh addition and asks them everything again.
        const isCoveredByOther = (path) => workspaces.some(w => isSubPath(w.path || w, path));
        
        Object.keys(customProjectNames).forEach(p => {
            if (isSubPath(workspaceToDelete, p) && !isCoveredByOther(p)) delete customProjectNames[p];
        });
        
        knownProjects = knownProjects.filter(p => !isSubPath(workspaceToDelete, p) || isCoveredByOther(p));
        hiddenProjects = hiddenProjects.filter(p => !isSubPath(workspaceToDelete, p) || isCoveredByOther(p));
        
        saveState();
        renderWorkspaces();
        scanAllWorkspaces(); // Re-evaluate allProjects based on remaining workspaces
        if (activeWorkspace === workspaceToDelete) {
            // Find if there's a parent workspace we can fall back to
            const parentWs = workspaces.find(w => {
                const wPath = w.path || w;
                return wPath !== workspaceToDelete && isSubPath(wPath, workspaceToDelete);
            });
            
            if (parentWs) {
                activeWorkspace = parentWs.path || parentWs;
                renderWorkspaces();
                renderProjects();
            } else {
                activeWorkspace = null;
                if (secondarySidebar) secondarySidebar.classList.add('collapsed');
            }
        }
    }
    deleteWorkspaceModal.style.display = 'none';
    workspaceToDelete = null;
});

const secondarySidebar = document.getElementById('secondary-sidebar');
const closeSecondarySidebarBtn = document.getElementById('close-secondary-sidebar-btn');

if (closeSecondarySidebarBtn) {
    closeSecondarySidebarBtn.addEventListener('click', () => {
        activeWorkspace = null;
        secondarySidebar.classList.add('collapsed');
        renderWorkspaces();
    });
}

// Initial state
if (secondarySidebar) secondarySidebar.classList.add('collapsed');

/// Hand the scan result to the backend.
///
/// Until this runs the gateway has no idea a project exists, so the phone can
/// only ever see servers that were already started at the desk.
async function persistScannedProjects() {
    try {
        await invoke('save_scanned_projects', { projects: allProjects });
    } catch (e) {
        console.error('Could not save the scanned projects', e);
    }
}

function saveState() {
    persist('workspaces', workspaces);
    persist('hiddenProjects', hiddenProjects);
    persist('knownProjects', knownProjects);
    persist('customProjectNames', customProjectNames);
    persist('pinnedProjects', pinnedProjects);
}

// Nothing may render until the database has answered, or the first paint
// would show an empty sidebar and then snap to the real workspaces.
bootstrapState().then(() => {
    maybeShowTour();
    updateMainIdeButton();
    renderWorkspaces();
    scanAllWorkspaces();
});

runCustomCmdBtn.addEventListener('click', async () => {
    if (!activeProject) return;
    const cmd = customCmdInput.value.trim();
    if (!cmd) return;
    
    try {
        await window.__TAURI__.core.invoke('run_custom_command', {
            projectPath: activeProject.path,
            commandStr: cmd
        });
        const processKey = `${activeProject.path}:$ ${cmd}`;
        runningProcesses.add(processKey);
        processLogs[processKey] = processLogs[processKey] || [];
        activeTerminalTab = processKey;
        customCmdInput.value = '';
        renderScripts();
        renderTerminalTabs();
    } catch (e) {
        await customAlert("Error running command: " + e);
    }
});

addWorkspaceBtn.addEventListener('click', async () => {
    const path = await window.__TAURI__.core.invoke('select_directory');
    if (path) {
        const pathStr = typeof path === 'string' ? path : path.path || path;
        if (!workspaces.includes(pathStr)) {
            scanProgressEl.style.display = 'block';
            scanProgressEl.textContent = 'Scanning new workspace...';
            try {
                const projects = await window.__TAURI__.core.invoke('scan_projects', { rootDir: pathStr });
                scanProgressEl.style.display = 'none';
                
                if (projects && projects.length > 0) {
                    tempScannedProjects = projects;
                    tempWorkspacePath = pathStr;
                    
                    // Render checkboxes
                    scanSelectionList.innerHTML = '';
                    tempScannedProjects.forEach(p => {
                        const div = document.createElement('div');
                        div.className = 'scan-selection-item';
                        div.style.padding = '0.5rem';
                        div.style.display = 'flex';
                        div.style.alignItems = 'center';
                        div.style.gap = '0.5rem';
                        div.style.borderBottom = '1px solid var(--border)';
                        
                        const cb = document.createElement('input');
                        cb.type = 'checkbox';
                        cb.checked = true; // Selected by default
                        cb.dataset.path = p.path;
                        cb.className = 'project-selection-checkbox';
                        
                        const label = document.createElement('div');
                        label.style.display = 'flex';
                        label.style.flexDirection = 'column';
                        
                        const nameSpan = document.createElement('span');
                        nameSpan.textContent = p.name;
                        nameSpan.style.fontWeight = '600';
                        
                        const pathSpan = document.createElement('span');
                        pathSpan.textContent = p.path;
                        pathSpan.style.fontSize = '0.75rem';
                        pathSpan.style.color = 'var(--text-muted)';
                        
                        label.appendChild(nameSpan);
                        label.appendChild(pathSpan);
                        
                        div.appendChild(cb);
                        div.appendChild(label);
                        scanSelectionList.appendChild(div);
                    });
                    
                    scanSelectionModal.style.display = 'flex';
                } else {
                    scanProgressEl.textContent = 'No projects found.';
                    setTimeout(() => scanProgressEl.style.display = 'none', 3000);
                }
            } catch (e) {
                scanProgressEl.style.display = 'none';
                console.error("Error scanning workspace:", e);
                await customAlert("Error scanning workspace: " + e);
            }
        } else {
            await customAlert("Workspace already exists.");
        }
    }
});

if (scanSelectionCancelBtn) {
    scanSelectionCancelBtn.addEventListener('click', () => {
        scanSelectionModal.style.display = 'none';
        tempScannedProjects = [];
        tempWorkspacePath = null;
    });
}

if (scanSelectionSaveBtn) {
    scanSelectionSaveBtn.addEventListener('click', () => {
        const checkboxes = document.querySelectorAll('.project-selection-checkbox');
        
        checkboxes.forEach(cb => {
            if (!cb.checked) {
                hiddenProjects.push(cb.dataset.path);
            }
        });
        
        persist('hiddenProjects', hiddenProjects);
        
        // Add workspace
        if (tempWorkspacePath && !workspaces.includes(tempWorkspacePath)) {
            workspaces.push(tempWorkspacePath);
            persist('workspaces', workspaces);
        }
        
        // Merge projects
        if (tempScannedProjects.length > 0) {
            const existingPaths = new Set(allProjects.map(p => p.path));
            const uniqueNewProjects = tempScannedProjects.filter(p => !existingPaths.has(p.path));
            allProjects = [...allProjects, ...uniqueNewProjects];
            persistScannedProjects();
        }
        
        scanSelectionModal.style.display = 'none';
        tempScannedProjects = [];
        tempWorkspacePath = null;
        
        updateTypeFilterDropdown();
        
        renderWorkspaces();
        renderProjects();
    });
}

if (scanSelectAllBtn) {
    scanSelectAllBtn.addEventListener('click', () => {
        document.querySelectorAll('.project-selection-checkbox').forEach(cb => cb.checked = true);
    });
}

if (scanUnselectAllBtn) {
    scanUnselectAllBtn.addEventListener('click', () => {
        document.querySelectorAll('.project-selection-checkbox').forEach(cb => cb.checked = false);
    });
}


const _ = window.__TAURI__.core.invoke;

// Tauri Event listeners
listen('process-output', (event) => {
    const { processKey, type, data } = event.payload;
    const cleanData = data.replace(/\x1B\[[0-9;]*[a-zA-Z]/g, '');
    const urlRegex = /(https?:\/\/[^\s\)'"\]]+)/g;
    const urls = cleanData.match(urlRegex);
    if (urls) {
        if (!processUrls[processKey]) processUrls[processKey] = new Set();
        let addedNew = false;
        urls.forEach(u => {
            if (u.includes('localhost') || u.includes('127.0.0.1') || u.includes('::1')) {
                if (!processUrls[processKey].has(u)) {
                    processUrls[processKey].add(u);
                    addedNew = true;
                }
            }
        });
        if (addedNew && activeProject) renderScripts();
    }
    appendLog(processKey, data, type === 'stderr');
});

listen('process-closed', (event) => {
    const { processKey, code } = event.payload;
    runningProcesses.delete(processKey);
    delete processUrls[processKey];
    processLogs[processKey] = processLogs[processKey] || [];
    processLogs[processKey].push({ text: `\n> Process exited with code ${code}\n`, isError: code !== 0 });
    if (activeProject) renderScripts();
    appendLog(processKey, `\n> Process exited with code ${code}\n`, code !== 0);
});

listen('scan-progress', (event) => {
    scanProgressEl.textContent = event.payload;
});

scanBtn.addEventListener('click', scanAllWorkspaces);

// Functions
function renderWorkspaces() {
    workspaceListEl.innerHTML = '';

    // Without this the sidebar is simply blank on a fresh install, with no
    // hint that "Add Workspace" is the next step.
    if (!workspaces.length) {
        workspaceListEl.innerHTML =
            '<div style="padding: 1.25rem 0.75rem; text-align: center; color: var(--text-muted);">' +
              '<svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" ' +
                'stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" style="opacity:0.5;margin-bottom:0.5rem;">' +
                '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path></svg>' +
              '<div style="font-size: 0.82rem; font-weight: 600; color: var(--text-primary);">No workspaces yet</div>' +
              '<div style="font-size: 0.75rem; margin-top: 0.25rem; line-height: 1.5;">' +
                'Click <strong>Add Workspace</strong> above and pick the folder your projects live in.' +
              '</div>' +
            '</div>';
        return;
    }

    workspaces.forEach(ws => {
        const div = document.createElement('div');
        div.className = 'workspace-item';
        div.style.display = 'flex';
        div.style.justifyContent = 'space-between';
        div.style.alignItems = 'center';
        div.title = ws.path || ws;
        
        const path = ws.path || ws;
        const name = path.split('\\').pop() || path;
        const textDiv = document.createElement('div');
        textDiv.style.overflow = 'hidden';
        textDiv.innerHTML = `<div class="project-name">${name}</div><div class="project-path">${path}</div>`;
        
        const removeBtn = document.createElement('button');
        removeBtn.className = 'btn-icon';
        removeBtn.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>';
        removeBtn.title = 'Remove Workspace';
        removeBtn.style.color = 'var(--danger)';
        removeBtn.onclick = (e) => {
            e.stopPropagation();
            workspaceToDelete = path;
            deleteWorkspaceModal.style.display = 'flex';
        };

        div.appendChild(textDiv);
        div.appendChild(removeBtn);
        
        if (activeWorkspace === path) {
            div.classList.add('active');
        }

        div.addEventListener('click', () => {
            activeWorkspace = path;
            renderWorkspaces();
            renderProjects();
            if (secondarySidebar) secondarySidebar.classList.remove('collapsed');
        });

        workspaceListEl.appendChild(div);
    });
}

async function scanAllWorkspaces() {
    projectListEl.innerHTML = '<div class="empty-state">Scanning...</div>';
    let tempProjects = [];
    
    for (let ws of workspaces) {
        let wsPath = typeof ws === 'string' ? ws : ws.path || ws;
        try {
            const projects = await window.__TAURI__.core.invoke('scan_projects', { rootDir: wsPath });
            if (projects && projects.length > 0) {
                tempProjects.push(...projects);
            }
        } catch (err) {
            console.error(`Failed to scan workspace ${wsPath}:`, err);
        }
    }
    
    // Sort projects to ensure consistent ordering
    tempProjects.sort((a, b) => a.name.localeCompare(b.name));
    
    // Find completely new projects that are not in knownProjects
    const newProjects = tempProjects.filter(p => !isPathInArray(knownProjects, p.path));
    
    allProjects = tempProjects;
    persistScannedProjects();
    
    // Auto-update filter dropdown
    updateTypeFilterDropdown();
    
    renderProjects();
    
    if (newProjects.length > 0) {
        startWorkspaceWizard(newProjects);
    }
}

function updateTypeFilterDropdown() {
    const types = new Set();
    allProjects.forEach(p => {
        const pt = p.project_type || 'Unknown';
        pt.split(', ').forEach(t => types.add(t));
    });
    const currentVal = projectTypeFilter.value;
    
    projectTypeFilter.innerHTML = '<option value="all">All Types</option>';
    Array.from(types).sort().forEach(t => {
        const opt = document.createElement('option');
        opt.value = t;
        opt.textContent = t;
        projectTypeFilter.appendChild(opt);
    });
    
    if (types.has(currentVal)) {
        projectTypeFilter.value = currentVal;
    } else {
        projectTypeFilter.value = 'all';
    }
}

function renderProjects() {
    projectListEl.innerHTML = '';
    
    if (!activeWorkspace) {
        projectListEl.innerHTML = '<div class="empty-state">Select a workspace to view projects.</div>';
        return;
    }

    if (allProjects.length === 0) {
        projectListEl.innerHTML = '<div class="empty-state">No projects found.</div>';
        return;
    }

    // Exclude projects that belong to a more specific sub-workspace
    const subWorkspaces = workspaces.filter(w => w !== activeWorkspace && isSubPath(activeWorkspace, w));
    console.log('[DEBUG] activeWorkspace:', activeWorkspace);
    console.log('[DEBUG] subWorkspaces:', subWorkspaces);
    console.log('[DEBUG] allProjects count:', allProjects.length);
    
    let visibleProjects = allProjects.filter(p => {
        if (isPathInArray(hiddenProjects, p.path) || !isSubPath(activeWorkspace, p.path)) return false;
        return !subWorkspaces.some(subWs => isSubPath(subWs, p.path));
    });
    console.log('[DEBUG] visibleProjects after filter:', visibleProjects.length);
    
    // Apply Filters
    const searchQuery = projectSearch.value.toLowerCase();
    const typeFilter = projectTypeFilter.value;
    
    if (searchQuery) {
        visibleProjects = visibleProjects.filter(p => {
            const name = customProjectNames[p.path] || p.name;
            return name.toLowerCase().includes(searchQuery);
        });
    }
    
    if (typeFilter !== 'all') {
        visibleProjects = visibleProjects.filter(p => {
            const pt = p.project_type || 'Unknown';
            return pt.split(', ').includes(typeFilter);
        });
    }

    if (visibleProjects.length === 0) {
        projectListEl.innerHTML = `<div class="empty-state">No matching projects found.</div>`;
    } else {
        // Sort by pinned status then by name
        visibleProjects.sort((a, b) => {
            const aPinned = pinnedProjects.includes(a.path);
            const bPinned = pinnedProjects.includes(b.path);
            if (aPinned && !bPinned) return -1;
            if (!aPinned && bPinned) return 1;
            const aName = customProjectNames[a.path] || a.name;
            const bName = customProjectNames[b.path] || b.name;
            return aName.localeCompare(bName);
        });

        visibleProjects.forEach(proj => {
            const div = document.createElement('div');
            const customName = customProjectNames[proj.path] || proj.name;
            div.className = 'project-item';
            div.style.display = 'grid';
            div.style.gridTemplateColumns = '1fr auto';
            div.style.alignItems = 'center';
            div.style.gap = '0.5rem';
            if (activeProject && activeProject.path === proj.path) {
                div.classList.add('active');
            }
            
            const textDiv = document.createElement('div');
            textDiv.style.overflow = 'hidden';
            textDiv.innerHTML = `<div class="project-name" style="white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${customName} <span class="badge" style="font-size:0.65rem; background:rgba(255,255,255,0.1); padding:2px 4px; border-radius:4px; margin-left:4px; display:inline-block; vertical-align:middle;">${proj.project_type || 'Unknown'}</span></div><div class="project-path" style="white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${proj.path}</div>`;
            
            const toggleBtn = document.createElement('button');
            toggleBtn.className = 'btn-icon hide-btn-hover';
            toggleBtn.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"></path><line x1="1" y1="1" x2="23" y2="23"></line></svg>';
            toggleBtn.title = 'Hide Project';
            toggleBtn.style.opacity = '0';
            toggleBtn.style.transition = 'opacity 0.2s';
            
            const isPinned = pinnedProjects.includes(proj.path);
            const pinBtn = document.createElement('button');
            pinBtn.className = 'btn-icon';
            pinBtn.innerHTML = `<svg width="16" height="16" viewBox="0 0 24 24" fill="${isPinned ? 'currentColor' : 'none'}" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="12" y1="17" x2="12" y2="22"></line><path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.68V6a3 3 0 0 0-3-3 3 3 0 0 0-3 3v4.68a2 2 0 0 1-1.11 1.87l-1.78.9A2 2 0 0 0 5 15.24Z"></path></svg>`;
            pinBtn.title = isPinned ? 'Unpin Project' : 'Pin Project';
            pinBtn.style.opacity = isPinned ? '1' : '0';
            pinBtn.style.transition = 'opacity 0.2s';
            pinBtn.style.color = isPinned ? 'var(--accent)' : 'inherit';

            const currentIde = getIdeInfo(defaultIde);
            const quickIdeBtn = document.createElement('button');
            quickIdeBtn.className = 'btn-icon quick-ide-btn';
            quickIdeBtn.innerHTML = currentIde.icon;
            quickIdeBtn.title = `Open in ${currentIde.name}`;
            quickIdeBtn.style.opacity = '0';
            quickIdeBtn.style.transition = 'opacity 0.2s';
            quickIdeBtn.onclick = async (e) => {
                e.stopPropagation();
                try {
                    await window.__TAURI__.core.invoke('open_in_editor', { path: proj.path, tool: defaultIde });
                } catch (err) {
                    await customAlert(`Could not launch ${currentIde.name}: ` + err);
                }
            };

            const actionsDiv = document.createElement('div');
            actionsDiv.style.display = 'flex';
            actionsDiv.style.gap = '0.25rem';
            actionsDiv.appendChild(quickIdeBtn);
            actionsDiv.appendChild(pinBtn);
            actionsDiv.appendChild(toggleBtn);
            
            div.onmouseenter = () => {
                toggleBtn.style.opacity = '1';
                pinBtn.style.opacity = '1';
                quickIdeBtn.style.opacity = '1';
            };
            div.onmouseleave = () => {
                toggleBtn.style.opacity = '0';
                quickIdeBtn.style.opacity = '0';
                if (!pinnedProjects.includes(proj.path)) pinBtn.style.opacity = '0';
            };
            
            toggleBtn.onclick = (e) => {
                e.stopPropagation();
                hiddenProjects.push(proj.path);
                saveState();
                renderProjects();
            };
            
            pinBtn.onclick = (e) => {
                e.stopPropagation();
                if (isPinned) {
                    pinnedProjects = pinnedProjects.filter(p => p !== proj.path);
                } else {
                    pinnedProjects.push(proj.path);
                }
                saveState();
                renderProjects();
            };

            div.appendChild(textDiv);
            div.appendChild(actionsDiv);
            
            div.addEventListener('click', () => {
                document.querySelectorAll('.project-item').forEach(el => el.classList.remove('active'));
                div.classList.add('active');
                selectProject(proj);
            });

            projectListEl.appendChild(div);
        });
    }


}

// Dependency Auto-Installer Modal Elements
const dependencyModal = document.getElementById('dependency-modal');
const depModalTitle = document.getElementById('dep-modal-title');
const depModalMessage = document.getElementById('dep-modal-message');
const depModalStatus = document.getElementById('dep-modal-status');
const depModalStatusText = document.getElementById('dep-modal-status-text');
const depModalInstallBtn = document.getElementById('dep-modal-install-btn');
const depModalLiveServerBtn = document.getElementById('dep-modal-liveserver-btn');
const depModalCancelBtn = document.getElementById('dep-modal-cancel-btn');

function showDependencyModal(toolName, scriptName, projectPath, scriptCmd) {
    return new Promise((resolve) => {
        if (!dependencyModal) return resolve('cancel');
        depModalTitle.textContent = `Missing ${toolName.toUpperCase()} Environment`;
        depModalMessage.textContent = `DevDeck detected that ${toolName.toUpperCase()} is required to run "${scriptName}", but it was not found on your system. Would you like DevDeck to install ${toolName.toUpperCase()} automatically via Windows Package Manager (Winget)?`;
        
        depModalStatus.style.display = 'none';
        depModalInstallBtn.style.display = 'flex';
        depModalLiveServerBtn.style.display = 'flex';
        depModalCancelBtn.style.display = 'flex';

        depModalInstallBtn.onclick = async () => {
            depModalStatus.style.display = 'block';
            depModalStatusText.textContent = `Installing ${toolName.toUpperCase()} via Winget... Please wait`;
            depModalInstallBtn.style.display = 'none';
            depModalLiveServerBtn.style.display = 'none';
            depModalCancelBtn.style.display = 'none';
            try {
                const res = await window.__TAURI__.core.invoke('auto_install_dependency', { tool: toolName });
                dependencyModal.style.display = 'none';
                await customAlert(`Successfully installed ${toolName.toUpperCase()}!\n\n${res}`);
                resolve('install');
            } catch (err) {
                dependencyModal.style.display = 'none';
                await customAlert(`Installation Error:\n${err}`);
                resolve('cancel');
            }
        };

        depModalLiveServerBtn.onclick = () => {
            dependencyModal.style.display = 'none';
            resolve('liveserver');
        };

        depModalCancelBtn.onclick = () => {
            dependencyModal.style.display = 'none';
            resolve('cancel');
        };

        dependencyModal.style.display = 'flex';
    });
}

async function executeScriptWithCheck(projectPath, scriptName, scriptCmd) {
    const processKey = `${projectPath}:${scriptName}`;
    if (runningProcesses.has(processKey)) return;

    let primaryBinary = '';
    const firstWord = scriptCmd.trim().split(/\s+/)[0].toLowerCase();
    if (['php', 'composer', 'python', 'cargo', 'go', 'rails', 'node'].includes(firstWord)) {
        primaryBinary = firstWord;
    }

    if (primaryBinary) {
        try {
            const isInstalled = await window.__TAURI__.core.invoke('check_system_dependency', { tool: primaryBinary });
            if (!isInstalled) {
                const userChoice = await showDependencyModal(primaryBinary, scriptName, projectPath, scriptCmd);
                if (userChoice === 'liveserver') {
                    scriptCmd = 'npx -y live-server';
                } else if (userChoice !== 'install') {
                    return;
                }
            }
        } catch (e) {
            console.error("Dependency check failed:", e);
        }
    }

    window.__TAURI__.core.invoke('run_script', { 
        projectPath: projectPath, 
        scriptName: scriptName,
        scriptCmd: scriptCmd
    });
    runningProcesses.add(processKey);
    processLogs[processKey] = processLogs[processKey] || [];
    activeTerminalTab = processKey;
    renderScripts();
    renderTerminalTabs();
}

function selectProject(proj) {
    activeProject = proj;
    window.__TAURI__.core.invoke('auto_setup_database', { projectPath: proj.path }).catch(() => {});
    const customName = customProjectNames[proj.path] || proj.name;
    activeProjectName.innerHTML = `${customName} <span class="badge" style="font-size:0.8rem; background:var(--primary); color:white; padding:2px 6px; border-radius:12px; margin-left:8px; vertical-align:middle;">${proj.project_type || 'Unknown'}</span>`;
    activeProjectPath.textContent = proj.path;
    scriptsSection.style.display = 'block';
    renameProjectBtn.style.display = 'block';
    openExternalTermBtn.style.display = 'block';
    // Sharing is per project, so the control only exists once one is selected.
    if (window.__devdeckShare) window.__devdeckShare.show(true);
    if (ideLauncherContainer) {
        ideLauncherContainer.style.display = 'block';
        updateMainIdeButton();
    }
    
    openExternalTermBtn.onclick = async () => {
        try {
            await window.__TAURI__.core.invoke('open_external_terminal', { path: proj.path });
        } catch (e) {
            await customAlert("Failed to open terminal: " + e);
        }
    };
    
    renameProjectBtn.onclick = async () => {
        const newName = await customPrompt("Enter new name for project:", customName);
        if (newName !== null && newName.trim() !== '') {
            customProjectNames[proj.path] = newName.trim();
            saveState();
            renderProjects();
            selectProject(proj);
        }
    };
    renderScripts();
    renderTerminalTabs();
}

function renderScripts() {
    scriptsGrid.innerHTML = '';
    if (!activeProject) return;

    const allScripts = { 'install': 'npm install', ...(activeProject.scripts || {}) };

    // Inject any running custom commands so they appear in the UI while active
    for (let pk of runningProcesses) {
        if (pk.startsWith(activeProject.path + ':$ ')) {
            const scriptName = pk.substring(activeProject.path.length + 1);
            if (!allScripts[scriptName]) {
                allScripts[scriptName] = scriptName.substring(2);
            }
        }
    }

    Object.entries(allScripts).forEach(([scriptName, scriptCmd]) => {
        const projectPath = activeProject.path;
        const processKey = `${projectPath}:${scriptName}`;
        const isRunning = runningProcesses.has(processKey);

        const container = document.createElement('div');
        container.className = 'script-btn-container';

        const runBtn = document.createElement('button');
        runBtn.className = `btn script-btn ${isRunning ? 'running' : ''}`;
        runBtn.title = scriptCmd;
        runBtn.innerHTML = `
            <span>${isRunning ? '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="spin"><line x1="12" y1="2" x2="12" y2="6"></line><line x1="12" y1="18" x2="12" y2="22"></line><line x1="4.93" y1="4.93" x2="7.76" y2="7.76"></line><line x1="16.24" y1="16.24" x2="19.07" y2="19.07"></line><line x1="2" y1="12" x2="6" y2="12"></line><line x1="18" y1="12" x2="22" y2="12"></line><line x1="4.93" y1="19.07" x2="7.76" y2="16.24"></line><line x1="16.24" y1="7.76" x2="19.07" y2="4.93"></line></svg>' : '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="5 3 19 12 5 21 5 3"></polygon></svg>'}</span>
            ${scriptName}
        `;
        runBtn.onclick = () => {
            executeScriptWithCheck(projectPath, scriptName, scriptCmd);
        };

        container.appendChild(runBtn);

        if (isRunning) {
            const stopBtn = document.createElement('button');
            stopBtn.className = 'btn stop-btn';
            stopBtn.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect></svg>';
            stopBtn.onclick = () => {
                window.__TAURI__.core.invoke('stop_script', { processKey });
            };
            container.appendChild(stopBtn);
            

        }

        scriptsGrid.appendChild(container);
    });
}

// Terminal UI
let activeTerminalTab = null;

function renderTerminalTabs() {
    terminalTabs.innerHTML = '';
    if (!activeProject) return;

    const keys = Object.keys(processLogs).filter(k => k.startsWith(activeProject.path + ':'));
    if (keys.length === 0) {
        terminalBody.innerHTML = '';
        activeTerminalTab = null;
        stdinInput.disabled = true;
        return;
    }

    if (!activeTerminalTab || !keys.includes(activeTerminalTab)) {
        activeTerminalTab = keys[0];
    }
    
    stdinInput.disabled = !runningProcesses.has(activeTerminalTab);

    keys.forEach(key => {
        const parts = key.split(':');
        const scriptName = parts[parts.length - 1];
        
        const tab = document.createElement('div');
        tab.className = `term-tab ${activeTerminalTab === key ? 'active' : ''}`;
        tab.style.display = 'flex';
        tab.style.alignItems = 'center';
        tab.style.gap = '0.5rem';
        
        const textSpan = document.createElement('span');
        textSpan.textContent = scriptName;
        tab.appendChild(textSpan);

        const closeBtn = document.createElement('span');
        closeBtn.innerHTML = '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>';
        closeBtn.style.display = 'flex';
        closeBtn.style.alignItems = 'center';
        closeBtn.style.cursor = 'pointer';
        closeBtn.style.opacity = '0.5';
        closeBtn.onmouseover = () => closeBtn.style.opacity = '1';
        closeBtn.onmouseout = () => closeBtn.style.opacity = '0.5';
        closeBtn.onclick = (e) => {
            e.stopPropagation();
            if (runningProcesses.has(key)) {
                window.__TAURI__.core.invoke('stop_script', { processKey: key });
            }
            delete processLogs[key];
            if (activeTerminalTab === key) activeTerminalTab = null;
            renderTerminalTabs();
        };
        tab.appendChild(closeBtn);

        tab.onclick = () => {
            activeTerminalTab = key;
            stdinInput.disabled = !runningProcesses.has(activeTerminalTab);
            renderTerminalTabs();
        };
        terminalTabs.appendChild(tab);
    });
    
    renderTerminalBody();
}

window.handleLinkClick = function(url, event) {
    window.__TAURI__.core.invoke('open_external_url', { url });
};

window.handleDirectLinkClick = function(url) {
    window.__TAURI__.core.invoke('open_external_url', { url });
};

window.handleKillProcess = function(pid, btnElement) {
    window.__TAURI__.core.invoke('kill_process', { pid: parseInt(pid, 10) });
    btnElement.textContent = "Killed!";
    btnElement.style.background = "#2ea043";
    btnElement.disabled = true;
};

function formatLogText(text) {
    let cleanText = text.replace(/\x1B\[[0-9;]*[a-zA-Z]/g, '');
    let escaped = cleanText.replace(/</g, '&lt;').replace(/>/g, '&gt;');
    const urlRegex = /(https?:\/\/[^\s\)'"\]]+)/g;
    escaped = escaped.replace(urlRegex, (url) => {
        return `<span onclick="window.handleLinkClick('${url}', event)" style="color: var(--accent); text-decoration: underline; cursor: pointer;" title="Click to open in browser">${url}</span>`;
    });
    
    const taskkillRegex = /Run taskkill \/PID (\d+) \/F to stop it\./gi;
    escaped = escaped.replace(taskkillRegex, (match, pid) => {
        return `${match} <button class="btn btn-sm" style="background:var(--danger); color:#fff; border:none; padding: 2px 8px; margin-left: 8px; font-size: 0.75rem; border-radius: 4px; cursor: pointer;" onclick="window.handleKillProcess('${pid}', this)">Force Kill PID ${pid}</button>`;
    });

    return escaped;
}

function renderTerminalBody() {
    terminalBody.innerHTML = '';
    if (!activeTerminalTab || !processLogs[activeTerminalTab]) return;

    const logs = processLogs[activeTerminalTab];
    let html = '';
    logs.forEach(logObj => {
        if (typeof logObj === 'string') {
            html += `<span class="log-line">${formatLogText(logObj)}</span>`;
        } else {
            html += `<span class="log-line ${logObj.isError ? 'log-err' : ''}">${formatLogText(logObj.text)}</span>`;
        }
    });
    terminalBody.innerHTML = html;
    terminalBody.scrollTop = terminalBody.scrollHeight;
}

function appendLog(processKey, text, isError = false) {
    processLogs[processKey] = processLogs[processKey] || [];
    processLogs[processKey].push({ text, isError });
    if (activeTerminalTab === processKey) {
        const span = document.createElement('span');
        span.className = `log-line ${isError ? 'log-err' : ''}`;
        span.innerHTML = formatLogText(text);
        terminalBody.appendChild(span);
        terminalBody.scrollTop = terminalBody.scrollHeight;
    }
}

// Sidebar Toggle Logic
const sidebar = document.querySelector('.primary-sidebar');
const sidebarToggleBtn = document.getElementById('sidebar-toggle');
const toggleIcon = document.getElementById('toggle-icon');

sidebarToggleBtn.addEventListener('click', () => {
    sidebar.classList.toggle('collapsed');
    if (sidebar.classList.contains('collapsed')) {
        toggleIcon.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6"></polyline></svg>';
        if (secondarySidebar) secondarySidebar.classList.add('collapsed');
    } else {
        toggleIcon.innerHTML = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="15 18 9 12 15 6"></polyline></svg>';
        if (activeWorkspace && secondarySidebar) secondarySidebar.classList.remove('collapsed');
    }
});

// Process Manager Logic
const processManagerBtn = document.getElementById('process-manager-btn');
const processModal = document.getElementById('process-modal');
const closeProcessModalBtn = document.getElementById('close-process-modal');
const processListEl = document.getElementById('process-list');
const refreshProcessesBtn = document.getElementById('refresh-processes-btn');

window.killProjectProcesses = function(projectPath, btnEl) {
    if (btnEl) {
        btnEl.disabled = true;
        btnEl.style.opacity = "0.7";
    }

    try {
        // Stop tracked scripts
        const scriptsToStop = Array.from(runningProcesses).filter(k => k.startsWith(projectPath + ':'));
        scriptsToStop.forEach(processKey => {
            window.__TAURI__.core.invoke('stop_script', { processKey });
        });
        
        // Kill lingering OS node processes
        const pids = window.currentProcesses.filter(p => p.projectPath === projectPath).map(p => p.pid);
        pids.forEach(pid => window.__TAURI__.core.invoke('kill_process', { pid }));
        
        setTimeout(loadAndRenderProcesses, 500);
    } catch (e) {
        console.error("Failed to kill project processes", e);
        setTimeout(loadAndRenderProcesses, 500);
    }
};

async function loadAndRenderProcesses() {
    processListEl.innerHTML = '<div style="text-align: center; padding: 2rem; color: var(--text-muted);">Loading processes...</div>';
    
    try {
        const processes = await window.__TAURI__.core.invoke('get_node_processes');
        window.currentProcesses = processes;
    
        if (processes.length === 0) {
            processListEl.innerHTML = '<div style="text-align: center; padding: 2rem; color: var(--text-muted);">No running Node.js processes found.</div>';
            return;
        }
        
        const grouped = {};
        processes.forEach(proc => {
            if (!grouped[proc.projectPath]) grouped[proc.projectPath] = [];
            grouped[proc.projectPath].push(proc);
        });
        
        let projectHtml = '';
        for (const [path, procs] of Object.entries(grouped)) {
            const folderName = path.split('\\').pop();
            projectHtml += `
                <div style="background: var(--surface); padding: 1rem; border-radius: 8px; border: 1px solid var(--border); margin-bottom: 0.5rem;">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.75rem;">
                        <div>
                            <div style="font-weight: 500; color: var(--text);">${folderName}</div>
                            <div style="font-size: 0.8rem; color: var(--text-muted);">${path}</div>
                        </div>
                        <button class="btn btn-primary kill-project-btn" data-path="${path.replace(/"/g, '&quot;')}" style="background: transparent; border: 1px solid var(--danger); color: var(--danger); transition: all 0.2s ease;" onmouseover="this.style.background='rgba(247, 118, 142, 0.1)'" onmouseout="this.style.background='transparent'">
                            Kill Project
                        </button>
                    </div>
                    <div style="font-size: 0.85rem; color: var(--text-muted);">
                        ${procs.length} process(es) running.
                    </div>
                </div>
            `;
        }
        processListEl.innerHTML = projectHtml;
        
        processListEl.querySelectorAll('.kill-project-btn').forEach(btn => {
            btn.onclick = () => killProjectProcesses(btn.getAttribute('data-path'), btn);
        });
    } catch(err) {
        processListEl.innerHTML = '<div class="empty-state">Error fetching processes: ' + err + '</div>';
    }
}

processManagerBtn.addEventListener('click', () => {
    processModal.style.display = 'flex';
    loadAndRenderProcesses();
});

closeProcessModalBtn.addEventListener('click', () => {
    processModal.style.display = 'none';
});

refreshProcessesBtn.addEventListener('click', () => {
    loadAndRenderProcesses();
});

// Close modal if clicking outside content
processModal.addEventListener('click', (e) => {
    if (e.target === processModal) {
        processModal.style.display = 'none';
    }
});


// Hidden Projects logic
manageHiddenBtn.addEventListener('click', () => {
    hiddenProjectSearch.value = '';
    renderHiddenProjectsList();
    hiddenProjectsModal.style.display = 'flex';
});

function renderHiddenProjectsList() {
    const term = hiddenProjectSearch.value.toLowerCase();
    const hiddenItems = allProjects.filter(p => isPathInArray(hiddenProjects, p.path) && p.name.toLowerCase().includes(term));
    hiddenProjectsList.innerHTML = '';
    
    if (hiddenItems.length === 0) {
        hiddenProjectsList.innerHTML = '<div class="text-muted" style="text-align: center; padding: 1rem;">No hidden projects found.</div>';
        return;
    }
    
    hiddenItems.forEach(proj => {
            const div = document.createElement('div');
            div.style.display = 'flex';
            div.style.justifyContent = 'space-between';
            div.style.alignItems = 'center';
            div.style.padding = '0.5rem';
            div.style.background = 'var(--surface-light)';
            div.style.borderRadius = '4px';
            
            const name = customProjectNames[proj.path] || proj.name;
            div.innerHTML = `<div><strong>${name}</strong> <span class="badge" style="background:var(--primary); color:white; padding: 2px 6px; border-radius: 12px; font-size: 0.7rem; margin-left: 0.5rem;">${proj.project_type || 'Unknown'}</span><br><small class="text-muted">${proj.path}</small></div>`;
            
            const restoreBtn = document.createElement('button');
            restoreBtn.className = 'btn btn-primary';
            restoreBtn.textContent = 'Restore';
            restoreBtn.onclick = () => {
                hiddenProjects = hiddenProjects.filter(p => p !== proj.path);
                saveState();
                div.remove();
                renderProjects();
            };
            
            div.appendChild(restoreBtn);
            hiddenProjectsList.appendChild(div);
        });
}

closeHiddenModalBtn.addEventListener('click', () => {
    hiddenProjectsModal.style.display = 'none';
});

// Filters
projectSearch.addEventListener('input', renderProjects);
projectTypeFilter.addEventListener('change', renderProjects);

// === ACCOUNT, SETTINGS AND SHARING ===
// Identity sits in the sidebar, device linking lives in Settings, and each
// project is shared from its own header rather than from a central panel.
(function initRemote() {
    const $ = (id) => document.getElementById(id);

    const accountModal  = $('account-modal');
    const settingsModal = $('settings-modal');
    if (!accountModal || !settingsModal) return;

    let signedIn = false;
    let gatewayInfo = null;
    let tunnelUrl = null;
    let previewRows = [];
    let settingsTimer = null;

    // ---------------------------------------------------------------- chip
    async function refreshAccountChip() {
        let st = { signed_in: false };
        try { st = await invoke('cloud_status'); } catch (e) {}
        signedIn = !!st.signed_in;

        const email = st.email || '';
        const letter = (email[0] || 'U').toUpperCase();
        $('account-name').textContent = signedIn ? (email.split('@')[0] || 'Account') : 'Sign in';
        $('account-sub').textContent = signedIn ? email : 'Not signed in';

        const avatar = $('account-avatar');
        if (signedIn) {
            avatar.textContent = letter;
            avatar.style.background = 'var(--accent)';
            avatar.style.color = '#fff';
            avatar.style.fontWeight = '700';
            avatar.style.fontSize = '0.78rem';
        }
        return st;
    }

    // ------------------------------------------------------- account modal
    let authMode = 'signin';

    function setAuthMode(mode) {
        authMode = mode;
        const isSignup = mode === 'signup';
        for (const t of document.querySelectorAll('.auth-tab')) {
            const on = t.dataset.tab === mode;
            t.style.background = on ? 'var(--accent)' : 'transparent';
            t.style.color = on ? '#fff' : 'var(--text-muted)';
            t.style.fontWeight = on ? '600' : '400';
        }
        // Re-entering the password only guards account creation; asking for it
        // at sign-in would be noise.
        $('au-pass2').style.display = isSignup ? 'block' : 'none';
        $('au-forgot').style.display = isSignup ? 'none' : 'inline';
        $('au-submit').textContent = isSignup ? 'Create account' : 'Sign in';
        $('au-msg').textContent = '';
    }

    async function openAccount() {
        const st = await refreshAccountChip();
        $('account-title').textContent = st.signed_in ? 'Account' : 'Sign in to DevDeck';
        $('auth-panes').style.display = st.signed_in ? 'none' : 'block';
        $('account-signed-in').style.display = st.signed_in ? 'block' : 'none';
        if (st.signed_in) {
            $('account-signed-email').textContent = st.email || '';
            $('account-big-avatar').textContent = ((st.email || 'U')[0] || 'U').toUpperCase();
        } else {
            setAuthMode('signin');
        }
        accountModal.style.display = 'flex';
    }

    function say(text, tone) {
        const el = $('au-msg');
        el.textContent = text;
        el.style.color = tone === 'bad' ? '#f87171'
                       : tone === 'good' ? 'var(--success, #22c55e)'
                       : 'var(--text-muted)';
    }

    $('account-chip').addEventListener('click', openAccount);
    $('close-account-modal').addEventListener('click', () => { accountModal.style.display = 'none'; });
    accountModal.addEventListener('click', (e) => { if (e.target === accountModal) accountModal.style.display = 'none'; });
    for (const t of document.querySelectorAll('.auth-tab')) {
        t.addEventListener('click', () => setAuthMode(t.dataset.tab));
    }

    $('au-eye').addEventListener('click', () => {
        const f = $('au-pass');
        const showing = f.type === 'text';
        f.type = showing ? 'password' : 'text';
        $('au-eye').title = showing ? 'Show password' : 'Hide password';
    });

    $('au-submit').addEventListener('click', async function () {
        const email = $('au-email').value.trim();
        const pass = $('au-pass').value;
        if (!email || !pass) { say('Enter your email and password.', 'bad'); return; }

        if (authMode === 'signup') {
            if (pass.length < 8) { say('Use at least 8 characters.', 'bad'); return; }
            if (pass !== $('au-pass2').value) { say('The two passwords do not match.', 'bad'); return; }
        }

        const original = this.textContent;
        this.disabled = true;
        this.textContent = authMode === 'signup' ? 'Creating…' : 'Signing in…';
        say('');
        try {
            if (authMode === 'signup') {
                const msg = await invoke('cloud_sign_up', { email: email, password: pass });
                // Stay on this screen and switch to sign-in, rather than parking
                // the user on a "waiting for confirmation" dead end.
                setAuthMode('signin');
                $('au-pass').value = pass;
                say(msg, 'good');
            } else {
                await invoke('cloud_sign_in', { email: email, password: pass });
                $('au-pass').value = '';
                $('au-pass2').value = '';
                await openAccount();
            }
        } catch (e) {
            say(String(e), 'bad');
        } finally {
            this.disabled = false;
            this.textContent = original;
        }
    });

    $('au-pass').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('au-submit').click(); });
    $('au-pass2').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('au-submit').click(); });

    $('au-forgot').addEventListener('click', async function () {
        const email = $('au-email').value.trim();
        if (!email) { say('Enter your email first, then press this.', 'bad'); return; }
        this.disabled = true;
        try { say(await invoke('cloud_reset_password', { email: email })); }
        catch (e) { say(String(e), 'bad'); }
        finally { this.disabled = false; }
    });

    $('au-signout').addEventListener('click', async () => {
        await invoke('cloud_sign_out');
        await openAccount();
    });

    // ------------------------------------------------------ settings modal
    async function refreshSettings() {
        try { gatewayInfo = await invoke('gateway_status'); } catch (e) { return; }

        const on = !!gatewayInfo.running;
        $('link-dot').style.background = on ? 'var(--success, #22c55e)' : 'var(--text-muted)';
        $('link-state').textContent = on ? 'Ready to link' : 'Sharing is off';
        $('gateway-toggle').textContent = on ? 'Turn off' : 'Turn on';

        let tunnels = {};
        try { tunnels = await invoke('tunnel_status'); } catch (e) {}
        tunnelUrl = (gatewayInfo.control_port && tunnels[gatewayInfo.control_port]) || null;

        $('link-url').value = tunnelUrl || gatewayInfo.lan_url || '';
        $('reach-label').textContent = tunnelUrl ? 'Reachable anywhere' : 'This Wi‑Fi only';
        $('reach-btn').textContent = tunnelUrl ? 'Stop' : 'Use anywhere';

        if (on) {
            try {
                $('link-qr').innerHTML = await invoke('gateway_pair_qr');
                const svg = $('link-qr').querySelector('svg');
                if (svg) { svg.style.width = '100%'; svg.style.height = '100%'; }
            } catch (e) { $('link-qr').textContent = String(e); }
        } else {
            $('link-qr').textContent = 'Sharing is off';
        }

        try { previewRows = await invoke('gateway_previews'); } catch (e) {}
        await refreshAccountChip();
    }

    function openSettings() {
        settingsModal.style.display = 'flex';
        invoke('gateway_start').catch(() => {}).then(refreshSettings);
        settingsTimer = setInterval(refreshSettings, 3000);
    }
    function closeSettings() {
        settingsModal.style.display = 'none';
        if (settingsTimer) { clearInterval(settingsTimer); settingsTimer = null; }
    }

    $('settings-btn').addEventListener('click', openSettings);
    $('close-settings-modal').addEventListener('click', closeSettings);
    settingsModal.addEventListener('click', (e) => { if (e.target === settingsModal) closeSettings(); });

    $('link-copy').addEventListener('click', () => {
        if (!$('link-url').value) return;
        navigator.clipboard.writeText($('link-url').value);
        $('link-copy').textContent = 'Copied';
        setTimeout(() => { $('link-copy').textContent = 'Copy'; }, 1200);
    });

    $('reach-btn').addEventListener('click', async function () {
        this.disabled = true;
        try {
            if (tunnelUrl) {
                await invoke('tunnel_stop', { port: gatewayInfo.control_port });
            } else {
                if (!(await invoke('tunnel_available'))) {
                    this.textContent = 'Installing…';
                    await invoke('tunnel_install');
                }
                this.textContent = 'Connecting…';
                await invoke('tunnel_start');
            }
            await refreshSettings();
        } catch (e) { customAlert(String(e)); }
        finally { this.disabled = false; }
    });

    $('gateway-toggle').addEventListener('click', async () => {
        const info = await invoke('gateway_status');
        if (info.running) { await invoke('gateway_stop'); } else { await invoke('gateway_start'); }
        await refreshSettings();
    });

    $('revoke-devices').addEventListener('click', async () => {
        await invoke('auth_revoke_sessions');
        customAlert('Every linked phone has been signed out.');
    });

    $('cloud-connect-btn').addEventListener('click', async () => {
        const url = $('cloud-url').value.trim();
        const key = $('cloud-key').value.trim();
        if (!url || !key) { customAlert('Paste both the project URL and the anon key.'); return; }
        try { await invoke('cloud_set_config', { url: url, anonKey: key }); await refreshSettings(); }
        catch (e) { customAlert(String(e)); }
    });

    // --------------------------------------------------- per-project share
    const sharePop = $('share-popover');

    /// The running preview for the active project, if any.
    function previewForActive() {
        if (!activeProject) return null;
        const path = (activeProject.path || '').replace(/[\\/]+$/, '').toLowerCase();
        return previewRows.find((r) => {
            const rp = r.key.slice(0, r.key.lastIndexOf(':')).replace(/[\\/]+$/, '').toLowerCase();
            return rp === path;
        }) || null;
    }

    async function refreshShare() {
        if (!activeProject) return;
        try { previewRows = await invoke('gateway_previews'); } catch (e) {}

        let vis = {};
        try { vis = await invoke('project_visibility'); } catch (e) {}
        const path = (activeProject.path || '').replace(/[\\/]+$/, '');
        const current = vis[path] || 'private';
        $('share-visibility').value = current;

        const row = previewForActive();
        const hint = $('share-hint');

        if (!row) {
            $('share-link').value = '';
            $('share-link').placeholder = 'Start the project first';
            hint.textContent = 'Run a script above, then a link appears here.';
            return;
        }

        // Prefer the address that works off this network.
        $('share-link').value = tunnelUrl
            ? tunnelUrl.replace(/\/$/, '') + '/p/' + row.port
            : row.url;

        if (current === 'public' && !tunnelUrl) {
            hint.textContent = 'Anyone with this link can open it, but only on this Wi-Fi. Turn on "Use anywhere" in Settings to share beyond it.';
        } else if (current === 'public') {
            hint.textContent = 'Anyone with this link can open it in a browser. No DevDeck app and no sign-in needed.';
        } else {
            hint.textContent = 'Only you. Whoever opens this must sign in with your account.';
        }
    }

    $('share-project-btn').addEventListener('click', async (e) => {
        e.stopPropagation();
        const showing = sharePop.style.display === 'block';
        sharePop.style.display = showing ? 'none' : 'block';
        if (!showing) {
            // The link is only meaningful once the gateway is up.
            invoke('gateway_start').catch(() => {}).then(async () => {
                let tunnels = {};
                try {
                    gatewayInfo = await invoke('gateway_status');
                    tunnels = await invoke('tunnel_status');
                } catch (err) {}
                tunnelUrl = (gatewayInfo && tunnels[gatewayInfo.control_port]) || null;
                await refreshShare();
            });
        }
    });

    document.addEventListener('click', (e) => {
        if (sharePop.style.display === 'block' && !e.target.closest('#share-container')) {
            sharePop.style.display = 'none';
        }
    });

    $('share-visibility').addEventListener('change', async () => {
        if (!activeProject) return;
        const path = (activeProject.path || '').replace(/[\\/]+$/, '');
        await invoke('set_project_visibility', {
            projectKey: path,
            public: $('share-visibility').value === 'public'
        });
        await refreshShare();
    });

    $('share-copy-btn').addEventListener('click', () => {
        const v = $('share-link').value;
        if (!v) return;
        navigator.clipboard.writeText(v);
        $('share-copy-btn').title = 'Copied';
        setTimeout(() => { $('share-copy-btn').title = 'Copy link'; }, 1200);
    });

    // Expose so project selection can reveal the share control.
    window.__devdeckShare = {
        show(on) {
            const c = $('share-container');
            if (c) c.style.display = on ? 'block' : 'none';
            if (!on) sharePop.style.display = 'none';
        }
    };

    refreshAccountChip();
})();
