// Import the WebAssembly module
import init, { 
    create_emulator_with_files, 
    set_button_state, 
    get_button_state, 
    get_flash_dump, 
    start_driving_emulator, 
    ir_connect, 
    ir_join, 
    ir_leave, 
    ir_code, 
    ir_connected, 
    ir_paired, 
    ir_status 
} from '/pkg/emiu2.js';

// Database utility for IndexedDB operations
const dbUtil = {
    dbName: 'emiu2DB',
    storeName: 'data',
    dbVersion: 1,
    
    openDB() {
        return new Promise((resolve, reject) => {
            const request = indexedDB.open(this.dbName, this.dbVersion);
            request.onupgradeneeded = event => {
                const db = event.target.result;
                if (!db.objectStoreNames.contains(this.storeName)) {
                    db.createObjectStore(this.storeName);
                }
            };
            request.onsuccess = () => resolve(request.result);
            request.onerror = () => reject(request.error);
        });
    },
    
    async save(key, arrayBuffer) {
        try {
            const db = await this.openDB();
            const transaction = db.transaction(this.storeName, 'readwrite');
            const store = transaction.objectStore(this.storeName);
            
            return new Promise((resolve, reject) => {
                const putRequest = store.put(arrayBuffer, key);
                putRequest.onsuccess = () => {
                    console.log(`Successfully saved ${key} to IndexedDB`);
                    resolve();
                };
                putRequest.onerror = () => reject(putRequest.error);
            });
        } catch (error) {
            console.error(`Failed to save ${key} to IndexedDB:`, error);
            throw error;
        }
    },
    
    async load(key) {
        try {
            const db = await this.openDB();
            const transaction = db.transaction(this.storeName, 'readonly');
            const store = transaction.objectStore(this.storeName);
            
            return new Promise((resolve, reject) => {
                const getRequest = store.get(key);
                getRequest.onsuccess = () => resolve(getRequest.result);
                getRequest.onerror = () => reject(getRequest.error);
            });
        } catch (error) {
            console.error(`Failed to load ${key} from IndexedDB:`, error);
            throw error;
        }
    }
};

// Error handling utility
async function executeWithErrorHandling(operation, errorMessage, onSuccess = () => {}) {
    try {
        const result = await operation();
        onSuccess(result);
        return result;
    } catch (error) {
        console.error(errorMessage, error);
        showNotification(errorMessage, 3000);
        return null;
    }
}

// UI Notification
function showNotification(message, duration = 3000) {
    const notification = document.getElementById('status-notification');
    notification.textContent = message;
    notification.classList.add('visible');
    
    setTimeout(() => {
        notification.classList.remove('visible');
    }, duration);
}

// Loading overlay management
function showLoading(message = 'Loading...') {
    const overlay = document.getElementById('loading-overlay');
    document.getElementById('loading-message').textContent = message;
    overlay.classList.add('visible');
}

function hideLoading() {
    const overlay = document.getElementById('loading-overlay');
    overlay.classList.remove('visible');
}

// File input display
function setupFileInputDisplay(inputId, nameId) {
    document.getElementById(inputId).addEventListener('change', function() {
        const fileName = this.files[0] ? this.files[0].name : 'No file chosen';
        document.getElementById(nameId).textContent = fileName;
    });
}

// Source selection handling
async function setupSourceSelection(type) {
    const sourceSelect = document.getElementById(`${type}-source-select`);
    const uploadDiv = document.getElementById(`${type}-upload`);
    const remoteDiv = document.getElementById(`${type}-remote`);
    const downloadDiv = type === 'flash' ? document.getElementById('download-flash-container') : null;
    
    // Configure initial state based on saved data
    async function updateInitialState() {
        try {
            const savedData = await dbUtil.load(type);
            const hasData = savedData && savedData.byteLength > 0;
            
            // Enable/disable saved option based on data availability
            sourceSelect.querySelector('option[value="db"]').disabled = !hasData;
            sourceSelect.value = hasData ? 'db' : 'remote';
            
            // Update display of related elements
            uploadDiv.style.display = 'none';
            remoteDiv.style.display = hasData ? 'none' : 'block';
            
            if (downloadDiv && type === 'flash') {
                // Ensure download button is visible when using saved flash
                downloadDiv.style.display = hasData ? 'block' : 'none';
            }
            
            return hasData;
        } catch (error) {
            console.error(`Error checking for saved ${type} data:`, error);
            sourceSelect.querySelector('option[value="db"]').disabled = true;
            sourceSelect.value = 'remote';
            uploadDiv.style.display = 'none';
            remoteDiv.style.display = 'block';
            if (downloadDiv) {
                downloadDiv.style.display = 'none';
            }
            return false;
        }
    }
    
    // Add change event listener
    sourceSelect.addEventListener('change', function() {
        const source = this.value;
        uploadDiv.style.display = source === 'file' ? 'block' : 'none';
        remoteDiv.style.display = source === 'remote' ? 'block' : 'none';
        
        if (downloadDiv) {
            downloadDiv.style.display = source === 'db' ? 'block' : 'none';
        }
    });
    
    return updateInitialState();
}

// UI Initialization
async function initializeUI() {
    // Make sure these are visible by default
    document.getElementById('otp-options').style.display = 'block';
    document.getElementById('flash-options').style.display = 'block';
    
    // Initialize source selection UI with proper visibility
    await Promise.all([
        setupSourceSelection('otp'),
        setupSourceSelection('flash')
    ]);
    
    // Add the file input name display setup
    setupFileInputDisplay('otp-input', 'otp-file-name');
    setupFileInputDisplay('flash-input', 'flash-file-name');
}

// Data fetching logic
async function fetchData(type) {
    const sourceSelect = document.getElementById(`${type}-source-select`);
    const source = sourceSelect.value;
    
    try {
        if (source === 'db') {
            const data = await dbUtil.load(type);
            if (!data || data.byteLength === 0) {
                throw new Error(`No saved ${type} found in database`);
            }
            return data;
        } 
        else if (source === 'remote') {
            const select = document.getElementById(`${type}-url-select`);
            const url = select.value;
            if (!url) {
                throw new Error(`Please select a ${type} from the dropdown`);
            }
            
            document.getElementById('start-button').disabled = true;
            document.getElementById('start-button').innerHTML = 
                `<span class="loading-spinner"></span> Downloading ${type}...`;
                
            const response = await fetch(url);
            if (!response.ok) {
                throw new Error(`Failed to fetch ${type}: ${response.status} ${response.statusText}`);
            }
            
            const data = await response.arrayBuffer();
            
            // Save OTP to database automatically
            if (type === 'otp') {
                await dbUtil.save(type, data);
            }
            
            return data;
        }
        else { // 'file'
            const fileInput = document.getElementById(`${type}-input`);
            if (fileInput.files.length === 0) {
                throw new Error(`Please select ${type} file`);
            }
            
            const data = await fileInput.files[0].arrayBuffer();
            
            // Save OTP to database automatically
            if (type === 'otp') {
                await dbUtil.save(type, data);
            }
            
            return data;
        }
    }
    catch (error) {
        console.error(`Error fetching ${type}:`, error);
        showNotification(error.message || `Error getting ${type} data`, 3000);
        return null;
    }
    finally {
        if (source === 'remote') {
            document.getElementById('start-button').disabled = false;
            document.getElementById('start-button').textContent = 'Start Emulator';
        }
    }
}

// Download saved flash
async function downloadSavedFlash() {
    const downloadButton = document.getElementById('download-flash');
    downloadButton.disabled = true;
    downloadButton.innerHTML = '<span class="loading-spinner"></span> Downloading...';
    
    await executeWithErrorHandling(
        async () => {
            const flashData = await dbUtil.load('flash');
            if (!flashData || flashData.byteLength === 0) {
                throw new Error('No flash data available');
            }
            
            const blob = new Blob([flashData], { type: 'application/octet-stream' });
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = 'emiu2_flash.bin';
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
            URL.revokeObjectURL(url);
            
            return flashData;
        },
        'Failed to download flash data',
        () => showNotification('Flash data downloaded successfully', 2000)
    );
    
    downloadButton.disabled = false;
    downloadButton.textContent = 'Download Saved Flash';
}

// Button state management
function update_button_colors(buttons) {
    for (const [elementId, buttonState] of Object.entries(buttons)) {
        const element = document.getElementById(elementId);
        if (get_button_state(buttonState)) {
            element.style.backgroundColor = 'var(--color-primary-active)';
            element.style.color = '#fff';
        } else {
            element.style.backgroundColor = elementId === 'power' ? 'var(--color-power)' : 'var(--color-button)';
            element.style.color = '#fff';
        }
    }
}

// Use requestAnimationFrame for better performance when updating button colors
let buttonUpdateScheduled = false;

function scheduleButtonUpdate(buttons) {
    if (!buttonUpdateScheduled) {
        buttonUpdateScheduled = true;
        requestAnimationFrame(() => {
            update_button_colors(buttons);
            buttonUpdateScheduled = false;
        });
    }
}

// Add special handling for power button to emphasize saving
function setupPowerButton() {
    const powerButton = document.getElementById('power');
    
    powerButton.addEventListener('mousedown', () => {
        // Add pulsing animation when power is pressed
        powerButton.style.animation = 'pulseShadow 1s 1';
    });
    
    powerButton.addEventListener('animationend', () => {
        powerButton.style.animation = '';
    });
}

// Show reset button when emulator starts
function showResetButton() {
    document.getElementById('reset-button').style.display = 'block';
}

// Better first-time guide detection
function showFirstTimeGuide() {
    // Convert localStorage check to use IndexedDB for consistency
    dbUtil.load('first-visit').then(visited => {
        if (!visited) {
            const guide = document.getElementById('first-use-guide');
            guide.classList.add('visible');
            
            document.getElementById('close-guide').addEventListener('click', () => {
                guide.classList.remove('visible');
                dbUtil.save('first-visit', new ArrayBuffer(1)); // Just save any data to mark as visited
            });
        }
    }).catch(() => {
        // If error reading, assume first visit
        const guide = document.getElementById('first-use-guide');
        guide.classList.add('visible');
        
        document.getElementById('close-guide').addEventListener('click', () => {
            guide.classList.remove('visible');
            dbUtil.save('first-visit', new ArrayBuffer(1));
        });
    });
}

// Store event handlers to allow proper cleanup
const eventHandlers = {
    keydown: null,
    keyup: null,
    buttonHandlers: []
};

// Main run function
async function run() {
    // Initialize UI immediately - this shows the configuration options
    await initializeUI();
    
    window.emulatorStarted = false;
    const buttons = {
        'up': 'up',
        'down': 'down',
        'left': 'left',
        'right': 'right',
        'menu': 'menu',
        'action': 'action',
        'power': 'power',
        'mute': 'mute',
        'screen-ul': 'screen-top-left',
        'screen-ur': 'screen-top-right',
        'screen-ll': 'screen-bottom-left',
        'screen-lr': 'screen-bottom-right'
    };

    // Show upload container with animation
    const uploadContainer = document.getElementById('upload-container');
    // Trigger reflow to ensure the transition works
    void uploadContainer.offsetWidth;
    uploadContainer.classList.add('visible');
    
    // Map keyboard keys to button states
    const keyMap = {
        'ArrowUp': 'up',
        'ArrowDown': 'down',
        'ArrowLeft': 'left',
        'ArrowRight': 'right',
        'm': 'menu',
        'M': 'menu',
        'a': 'action',
        'A': 'action',
        'p': 'power',
        'P': 'power',
        'u': 'mute',
        'U': 'mute'
    };
    update_button_colors(buttons);

    console.log('WASM module loaded');

    // Set initial canvas size
    const canvas = document.getElementById('emulator-canvas');
    canvas.width = 98;
    canvas.height = 67;

    document.getElementById('start-button').addEventListener('click', async () => {
        if (window.emulatorStarted) return;

        let otpArrayBuffer = null;
        let flashArrayBuffer = null;

        // Get OTP and Flash data
        otpArrayBuffer = await fetchData('otp');
        if (!otpArrayBuffer) return;
        
        flashArrayBuffer = await fetchData('flash');
        if (!flashArrayBuffer) return;

        try {
            showLoading('Starting emulator...');
            const otpUint8 = new Uint8Array(otpArrayBuffer);
            const flashUint8 = new Uint8Array(flashArrayBuffer);
            create_emulator_with_files(otpUint8, flashUint8);
            window.emulatorStarted = true;
            document.getElementById('start-button').disabled = true;
            
            // Disable select controls
            document.getElementById('otp-source-select').disabled = true;
            document.getElementById('flash-source-select').disabled = true;
            document.getElementById('otp-url-select').disabled = true;
            document.getElementById('flash-url-select').style.display = 'none';
            
            // Hide upload divs if they were visible
            document.getElementById('otp-upload').style.display = 'none';
            document.getElementById('flash-upload').style.display = 'none';
            
            // Hide upload container with animation
            const uploadContainer = document.getElementById('upload-container');
            uploadContainer.style.display = 'none';

            // Show controls with animation
            const controlsElement = document.getElementById('controls');
            controlsElement.style.display = 'grid';
            // Trigger reflow to ensure the transition works
            void controlsElement.offsetWidth;
            controlsElement.classList.add('visible');

            // Show the netplay panel now that an emulator exists.
            initializeNetplayUI();
            
            // Show reset button
            showResetButton();
            
            // Set up power button special handling
            setupPowerButton();
            
            // Scroll to top of the page to ensure emulator is visible
            window.scrollTo({ top: 0, behavior: 'smooth' });
            hideLoading();
        } catch (e) {
            hideLoading();
            showNotification('Failed to start emulator: ' + e, 5000);
            console.error(e);
            return;
        }

        const svg = document.getElementById('emiu2-svg');

        // Use the Web Animations API to get the current time of the CSS rotation
        const animations = svg.getAnimations();
        const cssAnim = animations.length > 0 ? animations[0] : null;
        const duration = 60000; // 60 seconds duration of the rotation animation
        let currentAngle = 360;
        if (cssAnim && cssAnim.currentTime != null) {
            let t = cssAnim.currentTime;
            currentAngle = 360 - ((t % duration) / duration * 360);
        }

        // Cancel any ongoing animations to freeze the current state
        svg.getAnimations().forEach(animation => animation.cancel());
        svg.style.transform = `translate(-50%, -50%) rotate(${currentAngle}deg)`;

        // Animate counterclockwise exit: subtract 720° from current angle
        const targetAngle = currentAngle - 720;
        const exitAnimation = svg.animate([
            { transform: `translate(-50%, -50%) rotate(${currentAngle}deg)` },
            { transform: `translate(-50%, -250%) rotate(${targetAngle}deg)` }
        ], {
            duration: 1000,
            easing: 'ease-in-out',
            fill: 'forwards'
        });

        exitAnimation.onfinish = async () => {
            try {
                start_driving_emulator();
            } catch (e) {
                showNotification('Failed to start emulator: ' + e, 5000);
                console.error(e);
            }
        };
    });

    // Add keyboard event listeners - using scheduleButtonUpdate for better performance
    eventHandlers.keydown = (event) => {
        const buttonState = keyMap[event.key];
        if (buttonState) {
            set_button_state(buttonState, true);
            scheduleButtonUpdate(buttons);
        }
    };
    
    eventHandlers.keyup = (event) => {
        const buttonState = keyMap[event.key];
        if (buttonState) {
            set_button_state(buttonState, false);
            scheduleButtonUpdate(buttons);
        }
    };
    
    document.addEventListener('keydown', eventHandlers.keydown);
    document.addEventListener('keyup', eventHandlers.keyup);

    // Add mouse/touch event listeners for on-screen buttons
    for (const [elementId, buttonState] of Object.entries(buttons)) {
        const element = document.getElementById(elementId);
        const on_events = ['mousedown', 'touchstart'];
        const off_events = ['mouseup', 'touchend', 'mouseleave', 'touchcancel'];

        on_events.forEach(eventType => {
            element.addEventListener(eventType, () => {
                set_button_state(buttonState, true);
                scheduleButtonUpdate(buttons);
            });
        });
        off_events.forEach(eventType => {
            element.addEventListener(eventType, () => {
                set_button_state(buttonState, false);
                scheduleButtonUpdate(buttons);
            });
        });
    }

    // Auto-save flash dump every 1 second
    setInterval(async () => {
        if (window.emulatorStarted) {
            try {
                const flashDump = get_flash_dump();
                if (flashDump && flashDump.buffer && flashDump.buffer.byteLength > 0) {
                    await dbUtil.save('flash', flashDump.buffer);
                    
                    // We're not showing a notification on every save since it happens frequently
                    // Only show notifications on errors
                }
            } catch (e) {
                console.error('Auto-save failed:', e);
                showNotification("Failed to save progress", 3000);
            }
        }
    }, 1000);

    // Add event listener for download flash button
    document.getElementById('download-flash').addEventListener('click', downloadSavedFlash);

    // Add reset button event listener
    document.getElementById('reset-button').addEventListener('click', async function() {
        if (confirm('Are you sure you want to reset the emulator? Any unsaved progress will be lost.')) {
            // Clean up event listeners first
            cleanupEventListeners();
            
            // Then reload
            window.location.reload();
        }
    });

    // Setup help toggle
    document.getElementById('help-toggle').addEventListener('click', function() {
        const expanded = this.getAttribute('aria-expanded') === 'true';
        this.setAttribute('aria-expanded', !expanded);
    });
}

// Add browser compatibility check at the start
function checkBrowserCompatibility() {
    const issues = [];
    
    // Check for WebAssembly support
    if (typeof WebAssembly === 'undefined') {
        issues.push('WebAssembly is not supported in this browser. Please try a modern browser like Chrome, Firefox, Safari, or Edge.');
    }
    
    // Check for IndexedDB support
    if (!window.indexedDB) {
        issues.push('IndexedDB is not supported in this browser, which is needed to save your progress.');
    }
    
    // Check for Web Audio API support
    if (!window.AudioContext && !window.webkitAudioContext) {
        issues.push('Web Audio API is not supported in this browser, which might affect sound playback.');
    }
    
    if (issues.length > 0) {
        // Show compatibility warning
        const warningEl = document.createElement('div');
        warningEl.className = 'browser-warning';
        warningEl.innerHTML = `
            <h3>Browser Compatibility Warning</h3>
            <ul>${issues.map(issue => `<li>${issue}</li>`).join('')}</ul>
            <button id="continue-anyway" class="btn btn-primary">Continue Anyway</button>
        `;
        document.body.appendChild(warningEl);
        
        // Return a promise that resolves when the user clicks "Continue Anyway"
        return new Promise(resolve => {
            document.getElementById('continue-anyway').addEventListener('click', () => {
                warningEl.remove();
                resolve();
            });
        });
    }
    
    return Promise.resolve();
}

// Update the initialize function to check compatibility first
async function initialize() {
    // Event listeners for touch interactions
    // Prevent context menu on long press for touch devices
    document.addEventListener('contextmenu', function(e) {
        if (e.target.closest('#controls')) {
            e.preventDefault();
        }
    }, false);

    // Prevent double-tap zoom on iOS
    document.addEventListener('touchend', function(e) {
        if (e.target.closest('#controls')) {
            e.preventDefault();
        }
    }, { passive: false });
    
    // Disable scrolling when guide is open
    const guideOverlay = document.getElementById('first-use-guide');
    const closeGuideButton = document.getElementById('close-guide');
    
    // Store original body style
    let originalBodyStyles = {
        overflow: '',
        position: '',
        width: '',
        height: '',
        top: ''
    };
    
    // Function to disable scrolling
    function disableScroll() {
        // Store current scroll position
        const scrollY = window.scrollY;
        
        // Save original styles
        originalBodyStyles.overflow = document.body.style.overflow;
        originalBodyStyles.top = document.body.style.top;
        
        // Set fixed position at current scroll
        document.body.style.overflow = 'hidden';
        document.body.style.top = `-${scrollY}px`;
    }
    
    // Function to enable scrolling
    function enableScroll() {
        // Get the scroll position from the body's top property
        const scrollY = parseInt(document.body.style.top || '0') * -1;
        
        // Restore original styles
        document.body.style.overflow = originalBodyStyles.overflow;
        document.body.style.top = originalBodyStyles.top;
        
        // Scroll back to the original position
        window.scrollTo(0, scrollY);
    }
    
    // Observe guide visibility changes
    const observer = new MutationObserver(function(mutations) {
        mutations.forEach(function(mutation) {
            if (mutation.attributeName === 'class') {
                if (guideOverlay.classList.contains('visible')) {
                    disableScroll();
                } else {
                    enableScroll();
                }
            }
        });
    });
    
    // Start observing the guide element
    observer.observe(guideOverlay, { attributes: true });
    
    // Handle close button click
    if (closeGuideButton) {
        closeGuideButton.addEventListener('click', enableScroll);
    }
    
    // Initial check
    if (guideOverlay.classList.contains('visible')) {
        disableScroll();
    }

    try {
        // Initialize the UI
        initializeUI();
        
        // Setup file inputs for better UX
        initializeFileInputs();
        
        // Check browser compatibility
        await checkBrowserCompatibility();
        
        showLoading('Loading emulator...');
        
        // Start rotating loading messages
        const messageInterval = rotateLoadingMessages();
        
        // Try to initialize WASM with a timeout
        const initPromise = init();
        const timeoutPromise = new Promise((_, reject) => 
            setTimeout(() => reject(new Error('Initialization timed out')), 15000)
        );
        
        await Promise.race([initPromise, timeoutPromise]);
        
        // Stop rotating messages
        clearInterval(messageInterval);
        
        console.log('WASM module initialized successfully');
        
        // Show first-time guide if needed
        showFirstTimeGuide();
        
        await run();
        hideLoading();
    } catch (e) {
        console.error('Failed to initialize the emulator:', e);
        hideLoading();
        
        showErrorMessage(e);
    }
}

// Start everything when the page loads
initialize();

// Add a cleanup function to remove event listeners before page reload or when resetting
function cleanupEventListeners() {
    if (eventHandlers.keydown) {
        document.removeEventListener('keydown', eventHandlers.keydown);
    }
    
    if (eventHandlers.keyup) {
        document.removeEventListener('keyup', eventHandlers.keyup);
    }
    
    eventHandlers.buttonHandlers.forEach(handler => {
        const { element, event, fn } = handler;
        element.removeEventListener(event, fn);
    });
    
    eventHandlers.buttonHandlers = [];
}

// Export public functions that may be needed
export {
    dbUtil,
    showNotification,
    showLoading,
    hideLoading
};

// Update error messages to be more user-friendly
function showErrorMessage(error) {
    const errorMessage = document.createElement('div');
    errorMessage.className = 'error-message';
    errorMessage.innerHTML = `
        <h3>Oops! Something went wrong</h3>
        <p>${error.message || 'The emulator encountered an unexpected issue'}</p>
        <p>This could be due to:</p>
        <ul>
            <li>Your browser may not support all required features</li>
            <li>The connection to load the emulator may have timed out</li>
            <li>There might be an issue with the selected firmware</li>
        </ul>
        <p>You can try:</p>
        <ol>
            <li>Using a different browser (Chrome or Firefox recommended)</li>
            <li>Checking your internet connection</li>
            <li>Reloading the page</li>
        </ol>
        <button id="retry-button" class="btn btn-primary">Try Again</button>
    `;
    document.body.appendChild(errorMessage);
    
    document.getElementById('retry-button').addEventListener('click', () => {
        window.location.reload();
    });
}

// More informative loading messages
const loadingMessages = [
    'Loading emulator components...',
    'Setting up virtual hardware...',
    'Preparing display system...',
    'Almost ready...'
];

function rotateLoadingMessages() {
    let messageIndex = 0;
    const messageElement = document.getElementById('loading-message');
    
    return setInterval(() => {
        messageElement.textContent = loadingMessages[messageIndex];
        messageIndex = (messageIndex + 1) % loadingMessages.length;
    }, 2500);
}

// Clean up the file input handling function
function initializeFileInputs() {
    const canvas = document.createElement('canvas');
    const context = canvas.getContext('2d');
    
    function getTextWidth(text, font) {
        context.font = font || getComputedStyle(document.body).font;
        return context.measureText(text).width;
    }
    
    function truncateFilename(filename, maxWidth, element) {
        const style = window.getComputedStyle(element);
        const font = style.font;
        const paddingLeft = parseFloat(style.paddingLeft);
        const paddingRight = parseFloat(style.paddingRight);
        const iconWidth = 25;
        const availableWidth = maxWidth - paddingLeft - paddingRight - iconWidth;
        
        if (getTextWidth(filename, font) <= availableWidth) {
            return filename;
        }
        
        const extension = filename.lastIndexOf('.') > 0 ? 
            filename.substring(filename.lastIndexOf('.')) : '';
        const nameWithoutExt = filename.substring(0, filename.length - extension.length);
        
        let truncatedName = nameWithoutExt;
        const ellipsis = '...';
        
        while (truncatedName.length > 1 && 
               getTextWidth(truncatedName + ellipsis + extension, font) > availableWidth) {
            truncatedName = truncatedName.substring(0, truncatedName.length - 1);
        }
        
        return truncatedName + ellipsis + extension;
    }
    
    function updateFilenameDisplay(input, label) {
        if (input.files.length > 0) {
            const filename = input.files[0].name;
            const labelWidth = label.offsetWidth;
            
            label.textContent = truncateFilename(filename, labelWidth, label);
            label.title = filename;
            label.classList.add('has-file');
        } else {
            label.textContent = label === otpLabel ? 'Choose OTP File' : 'Choose Flash File';
            label.title = '';
            label.classList.remove('has-file');
        }
    }
    
    const otpInput = document.getElementById('otp-input');
    const otpLabel = document.getElementById('otp-input-label');
    
    otpInput.addEventListener('change', function() {
        updateFilenameDisplay(this, otpLabel);
    });
    
    const flashInput = document.getElementById('flash-input');
    const flashLabel = document.querySelector('label[for="flash-input"]');
    
    flashInput.addEventListener('change', function() {
        updateFilenameDisplay(this, flashLabel);
    });
    
    window.addEventListener('resize', function() {
        if (otpInput.files.length > 0) {
            updateFilenameDisplay(otpInput, otpLabel);
        }
        if (flashInput.files.length > 0) {
            updateFilenameDisplay(flashInput, flashLabel);
        }
    });
}

// Ensure this is called appropriately
initializeFileInputs(); 
// ---------------------------------------------------------------------------
// Netplay: IR over the internet through an emiu2 relay server.

function initializeNetplayUI() {
    const panel = document.getElementById('netplay');
    const urlInput = document.getElementById('relay-url');
    const connectButton = document.getElementById('netplay-connect');
    const pairingRow = document.getElementById('netplay-pairing');
    const codeSpan = document.getElementById('netplay-code');
    const friendInput = document.getElementById('friend-code');
    const joinButton = document.getElementById('netplay-join');
    const leaveButton = document.getElementById('netplay-leave');
    const statusLine = document.getElementById('netplay-status');

    // A sensible default: same host as the page, the relay's default
    // port, wss when the page itself is secure (required by browsers).
    if (!urlInput.value) {
        const scheme = location.protocol === 'https:' ? 'wss' : 'ws';
        const host = location.hostname || 'localhost';
        urlInput.value = `${scheme}://${host}:5885`;
    }

    connectButton.addEventListener('click', () => {
        try {
            ir_connect(urlInput.value.trim());
        } catch (e) {
            statusLine.textContent = String(e);
        }
    });

    joinButton.addEventListener('click', () => {
        try {
            ir_join(friendInput.value.trim());
        } catch (e) {
            statusLine.textContent = String(e);
        }
    });

    leaveButton.addEventListener('click', () => {
        try {
            ir_leave();
        } catch (e) {
            statusLine.textContent = String(e);
        }
    });

    setInterval(() => {
        statusLine.textContent = ir_status();
        const connected = ir_connected();
        const paired = ir_paired();
        pairingRow.style.display = connected ? 'flex' : 'none';
        codeSpan.textContent = ir_code();
        joinButton.style.display = paired ? 'none' : '';
        friendInput.style.display = paired ? 'none' : '';
        leaveButton.style.display = paired ? '' : 'none';
    }, 500);

    panel.style.display = 'block';
}
