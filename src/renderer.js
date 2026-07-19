const { invoke } = window.__TAURI__.tauri;
const { listen } = window.__TAURI__.event;
const { open } = window.__TAURI__.dialog;

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

let workspaces = JSON.parse(localStorage.getItem('workspaces') || '[]');
let allProjects = [];
let hiddenProjects = JSON.parse(localStorage.getItem('hiddenProjects') || '[]');
let knownProjects = JSON.parse(localStorage.getItem('knownProjects') || '[]');
let customProjectNames = JSON.parse(localStorage.getItem('customProjectNames') || '{}');
let pinnedProjects = JSON.parse(localStorage.getItem('pinnedProjects') || '[]');
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


if (localStorage.getItem('tourCompleted') !== 'true') {
    tourWelcomeModal.style.display = 'flex';
}

document.getElementById('skip-tour-btn').addEventListener('click', () => {
    localStorage.setItem('tourCompleted', 'true');
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
            localStorage.removeItem('tourCompleted');
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
    
    localStorage.setItem('tourCompleted', 'true');
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

// Pre-populate with scratch directory as a default workspace if empty
const defaultWorkspace = "C:\\Users\\upend\\.gemini\\antigravity\\scratch";
if (workspaces.length === 0) {
    workspaces.push(defaultWorkspace);
    saveState();
}

function saveState() {
    localStorage.setItem('workspaces', JSON.stringify(workspaces));
    localStorage.setItem('hiddenProjects', JSON.stringify(hiddenProjects));
    localStorage.setItem('knownProjects', JSON.stringify(knownProjects));
    localStorage.setItem('customProjectNames', JSON.stringify(customProjectNames));
    localStorage.setItem('pinnedProjects', JSON.stringify(pinnedProjects));
}

renderWorkspaces();
scanAllWorkspaces();

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
        
        localStorage.setItem('hiddenProjects', JSON.stringify(hiddenProjects));
        
        // Add workspace
        if (tempWorkspacePath && !workspaces.includes(tempWorkspacePath)) {
            workspaces.push(tempWorkspacePath);
            localStorage.setItem('workspaces', JSON.stringify(workspaces));
        }
        
        // Merge projects
        if (tempScannedProjects.length > 0) {
            const existingPaths = new Set(allProjects.map(p => p.path));
            const uniqueNewProjects = tempScannedProjects.filter(p => !existingPaths.has(p.path));
            allProjects = [...allProjects, ...uniqueNewProjects];
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
const listen = window.__TAURI__.event.listen;

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

            const actionsDiv = document.createElement('div');
            actionsDiv.style.display = 'flex';
            actionsDiv.style.gap = '0.25rem';
            actionsDiv.appendChild(pinBtn);
            actionsDiv.appendChild(toggleBtn);
            
            div.onmouseenter = () => {
                toggleBtn.style.opacity = '1';
                pinBtn.style.opacity = '1';
            };
            div.onmouseleave = () => {
                toggleBtn.style.opacity = '0';
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

function selectProject(proj) {
    activeProject = proj;
    const customName = customProjectNames[proj.path] || proj.name;
    activeProjectName.innerHTML = `${customName} <span class="badge" style="font-size:0.8rem; background:var(--primary); color:white; padding:2px 6px; border-radius:12px; margin-left:8px; vertical-align:middle;">${proj.project_type || 'Unknown'}</span>`;
    activeProjectPath.textContent = proj.path;
    scriptsSection.style.display = 'block';
    renameProjectBtn.style.display = 'block';
    
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
            if (!isRunning) {
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
        return;
    }

    if (!activeTerminalTab || !keys.includes(activeTerminalTab)) {
        activeTerminalTab = keys[0];
    }

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
