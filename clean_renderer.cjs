const fs = require('fs');

let content = fs.readFileSync('e:/tauri-project-manager/src/renderer.js', 'utf8');

// 1. Remove knownProjects and wizard state variables
content = content.replace(/let knownProjects = JSON\.parse\(localStorage\.getItem\('knownProjects'\) \|\| '\[\]'\);\r?\n/, '');
content = content.replace(/const wizardTitle = document\.getElementById\('wizard-title'\);\r?\nconst wizardSubtitle = document\.getElementById\('wizard-subtitle'\);\r?\nconst wizardCancelAllBtn = document\.getElementById\('wizard-cancel-all-btn'\);\r?\n\r?\n\/\/ Wizard State Variables\r?\nlet wizardWorkspaces = \[\];\r?\nlet wizardCurrentStep = 0;\r?\nlet wizardGroupedProjects = \{\};\r?\n/, '');

// 2. Remove knownProjects save logic
content = content.replace(/    localStorage\.setItem\('knownProjects', JSON\.stringify\(knownProjects\)\);\r?\n/, '');

// 3. Update addWorkspaceBtn
const addWsRegex = /                    allProjects = \[\.\.\.allProjects, \.\.\.uniqueNewProjects\];\r?\n[\s\S]*?(?=                \}\r?\n            \}\r?\n        \}\r?\n    \}\)\r?\n\})/m;
content = content.replace(addWsRegex, "                    allProjects = [...allProjects, ...uniqueNewProjects];\n                    \n                    // Silently add the workspace and refresh\n                    if (!workspaces.includes(pathStr)) {\n                        workspaces.push(pathStr);\n                        localStorage.setItem('workspaces', JSON.stringify(workspaces));\n                        renderWorkspaces();\n                    }\n                    renderProjects();\n");

// 4. Remove scanAllBtn listener
const scanAllRegex = /scanAllBtn\.addEventListener\('click', async \(\) => \{[\s\S]*?\}\);\r?\n/m;
content = content.replace(scanAllRegex, '');

// 5. Update scanAllWorkspaces (remove newProjects / wizard logic)
const scanAllWsRegex = /    \/\/ Find completely new projects that are not in knownProjects\r?\n    const newProjects = tempProjects\.filter\(p => !isPathInArray\(knownProjects, p\.path\)\);\r?\n    \r?\n    allProjects = tempProjects;\r?\n    \r?\n    \/\/ Auto-update filter dropdown\r?\n    updateTypeFilterDropdown\(\);\r?\n    \r?\n    if \(newProjects\.length > 0\) \{\r?\n        startWorkspaceWizard\(newProjects\);\r?\n    \}\r?\n\}/m;
content = content.replace(scanAllWsRegex, "    allProjects = tempProjects;\n    updateTypeFilterDropdown();\n}");

// 6. Remove the wizard functions and event listeners
const wizardBlockRegex = /\/\/ Scan Modal Logic[\s\S]*?wizardCancelAllBtn\.addEventListener\('click', \(\) => \{[\s\S]*?renderProjects\(\);\r?\n\}\);\r?\n/m;
content = content.replace(wizardBlockRegex, '');

fs.writeFileSync('e:/tauri-project-manager/src/renderer.js', content);
console.log("Cleaned renderer.js");
