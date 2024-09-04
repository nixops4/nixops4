# cargo_or_provided_exe
#
# Synopsis:
# $(eval $(call cargo_or_provided_exe,VAR_PREFIX,CRATE_NAME))
#
# Define variables and/or rules for a crate, to build it with Cargo or get a command from PATH.
#
# VAR_PREFIX: Prefix for the variables to be defined
#               - $(VAR_PREFIX)_EXE: Path to the executable, or empty. Use this as a dependency when running the $(VAR_PREFIX)_CMD
#               - $(VAR_PREFIX)_CMD: Command to run the executable, or the provided command
# CRATE_NAME: Name of the crate and the executable
define cargo_or_provided_exe
ifeq ($(shell test -e ../../rust/$(2)/Cargo.toml && echo yes),yes)
$(1)_EXE = ../../rust/target/debug/$(2)
$(1)_CMD = $$($(1)_EXE)

$$($(1)_EXE): ../../rust/$(2)/Cargo.toml $(shell find ../../rust/$(2)/src -type f)
	@echo "       CARGO" $$@
	@cargo build --manifest-path ../../rust/$(2)/Cargo.toml
else
$(1)_EXE =
$(1)_CMD = $(2)
endif
endef
