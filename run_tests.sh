#!/bin/bash

# Comprehensive test runner for crabmux
# This script runs all tests and provides a summary

set -e

echo "🦀 Crabmux Comprehensive Test Suite"
echo "===================================="
echo

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to run tests and capture results
run_test_category() {
    local category="$1"
    local command="$2"
    
    echo -e "${BLUE}Running $category tests...${NC}"
    if eval "$command" > /dev/null 2>&1; then
        echo -e "${GREEN}✓ $category tests passed${NC}"
        return 0
    else
        echo -e "${RED}✗ $category tests failed${NC}"
        return 1
    fi
}

# Test categories
declare -a test_names=("Unit_Tests" "Session_Parsing" "CLI_Commands" "Error_Handling" "File_Operations" "Integration_Tests")
declare -a test_commands=(
    "cargo test --bin cmux"
    "cargo test --test parsing_tests"
    "cargo test --test cli_tests" 
    "cargo test --test error_handling_tests"
    "cargo test --test file_operations_tests"
    "cargo test --test integration_tests"
)

# Run all test categories
failed_tests=()
passed_tests=()

for i in "${!test_names[@]}"; do
    category="${test_names[$i]}"
    command="${test_commands[$i]}"
    display_name="${category//_/ }"
    
    if run_test_category "$display_name" "$command"; then
        passed_tests+=("$display_name")
    else
        failed_tests+=("$display_name")
    fi
done

echo
echo "Test Summary"
echo "============"

if [ ${#passed_tests[@]} -gt 0 ]; then
    echo -e "${GREEN}Passed (${#passed_tests[@]}):${NC}"
    for test in "${passed_tests[@]}"; do
        echo -e "  ${GREEN}✓${NC} $test"
    done
fi

if [ ${#failed_tests[@]} -gt 0 ]; then
    echo -e "${RED}Failed (${#failed_tests[@]}):${NC}"
    for test in "${failed_tests[@]}"; do
        echo -e "  ${RED}✗${NC} $test"
    done
fi

echo
if [ ${#failed_tests[@]} -eq 0 ]; then
    echo -e "${GREEN}🎉 All tests passed! (${#passed_tests[@]}/${#test_names[@]} categories)${NC}"
    exit 0
else
    echo -e "${RED}❌ Some tests failed (${#failed_tests[@]}/${#test_names[@]} categories failed)${NC}"
    echo
    echo "To run specific failing tests, use the commands that failed above."
    exit 1
fi