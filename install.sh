#!/bin/bash

# crabmux installer script
# Usage: curl -sSL https://raw.githubusercontent.com/yourusername/crabmux/main/install.sh | bash

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Detect OS and architecture
detect_platform() {
    local os
    local arch
    
    # Detect OS
    case "$(uname -s)" in
        Linux*)     os="linux" ;;
        Darwin*)    os="macos" ;;
        CYGWIN*|MINGW*|MSYS*) os="windows" ;;
        *)          
            print_error "Unsupported operating system: $(uname -s)"
            exit 1
            ;;
    esac
    
    # Detect architecture
    case "$(uname -m)" in
        x86_64|amd64)   arch="x86_64" ;;
        aarch64|arm64)  arch="aarch64" ;;
        *)              
            print_error "Unsupported architecture: $(uname -m)"
            exit 1
            ;;
    esac
    
    echo "${os}-${arch}"
}

# Get latest release version
get_latest_version() {
    local api_url="https://api.github.com/repos/yourusername/crabmux/releases/latest"
    
    # Try to get version from GitHub API
    if command -v curl >/dev/null 2>&1; then
        curl -s "$api_url" | grep '"tag_name":' | sed -E 's/.*"tag_name": "([^"]+)".*/\1/' | head -1
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$api_url" | grep '"tag_name":' | sed -E 's/.*"tag_name": "([^"]+)".*/\1/' | head -1
    else
        print_error "Neither curl nor wget is available. Please install one of them."
        exit 1
    fi
}

# Download and install crabmux
install_crabmux() {
    local platform="$1"
    local version="$2"
    local install_dir="${3:-/usr/local/bin}"
    
    # Construct download URL
    local binary_name="cmux-${platform}"
    if [[ "$platform" == *"windows"* ]]; then
        binary_name="${binary_name}.exe"
    fi
    
    local download_url="https://github.com/yourusername/crabmux/releases/download/${version}/${binary_name}"
    local temp_file="/tmp/${binary_name}"
    
    print_status "Downloading crabmux ${version} for ${platform}..."
    
    # Download the binary
    if command -v curl >/dev/null 2>&1; then
        curl -L -o "$temp_file" "$download_url"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$temp_file" "$download_url"
    else
        print_error "Neither curl nor wget is available."
        exit 1
    fi
    
    # Verify download
    if [[ ! -f "$temp_file" ]]; then
        print_error "Failed to download crabmux binary"
        exit 1
    fi
    
    # Make executable
    chmod +x "$temp_file"
    
    # Install to system path
    local target_file="${install_dir}/cmux"
    
    print_status "Installing to ${target_file}..."
    
    # Try to install to system directory
    if [[ -w "$install_dir" ]]; then
        mv "$temp_file" "$target_file"
    else
        # Use sudo if directory is not writable
        print_status "Requesting sudo permission to install to ${install_dir}..."
        sudo mv "$temp_file" "$target_file"
    fi
    
    print_success "crabmux installed successfully!"
}

# Verify installation
verify_installation() {
    if command -v cmux >/dev/null 2>&1; then
        local installed_version
        installed_version=$(cmux --version 2>/dev/null | head -1 || echo "unknown")
        print_success "Installation verified: ${installed_version}"
        print_status "You can now use 'cmux' to manage your tmux sessions!"
        print_status ""
        print_status "Quick start:"
        print_status "  cmux           # Interactive mode"
        print_status "  cmux ls        # List sessions"
        print_status "  cmux n myapp   # Create new session"
        print_status "  cmux a myapp   # Attach to session"
        print_status ""
        print_status "For more information, run: cmux --help"
    else
        print_error "Installation verification failed. cmux command not found in PATH."
        print_warning "You may need to restart your shell or update your PATH."
        return 1
    fi
}

# Check prerequisites
check_prerequisites() {
    print_status "Checking prerequisites..."
    
    # Check if tmux is installed
    if ! command -v tmux >/dev/null 2>&1; then
        print_warning "tmux is not installed or not in PATH."
        print_status "crabmux requires tmux to function. Please install tmux first:"
        print_status ""
        print_status "  # macOS"
        print_status "  brew install tmux"
        print_status ""
        print_status "  # Ubuntu/Debian"
        print_status "  sudo apt-get install tmux"
        print_status ""
        print_status "  # Fedora"
        print_status "  sudo dnf install tmux"
        print_status ""
        print_status "  # Arch"
        print_status "  sudo pacman -S tmux"
        print_status ""
        
        read -p "Do you want to continue installation anyway? (y/N): " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            print_status "Installation cancelled."
            exit 0
        fi
    else
        print_success "tmux is installed: $(tmux -V)"
    fi
}

# Main installation function
main() {
    print_status "crabmux installer"
    print_status "=================="
    
    # Check prerequisites
    check_prerequisites
    
    # Detect platform
    local platform
    platform=$(detect_platform)
    print_status "Detected platform: ${platform}"
    
    # Get latest version
    local version
    version=$(get_latest_version)
    if [[ -z "$version" ]]; then
        print_error "Failed to get latest version information"
        exit 1
    fi
    print_status "Latest version: ${version}"
    
    # Determine install directory
    local install_dir="/usr/local/bin"
    if [[ ":$PATH:" != *":$install_dir:"* ]]; then
        # Try alternative directories if /usr/local/bin is not in PATH
        for dir in "$HOME/.local/bin" "$HOME/bin" "/usr/bin"; do
            if [[ ":$PATH:" == *":$dir:"* ]] && [[ -d "$dir" ]]; then
                install_dir="$dir"
                break
            fi
        done
    fi
    
    # Install crabmux
    install_crabmux "$platform" "$version" "$install_dir"
    
    # Verify installation
    verify_installation
}

# Run main function
main "$@"