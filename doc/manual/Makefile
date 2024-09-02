
default: book

.PHONY: clean default open

open: book
	xdg-open book/index.html || open book/index.html

GENERATED = src/schema/resource-schema-v0.gen.md \
	src/cli/nixops4-resource-runner.md \
	src/schema/resource-v0/examples/CreateResourceRequest.json \
	src/schema/resource-v0/examples/CreateResourceResponse.json

clean:
	rm -rf book $(GENERATED)

book: $(shell find src -type f) $(GENERATED)
	@echo "      MDBOOK" $@
	@mdbook build

src/schema/resource-schema-v0.json: ../../rust/nixops4-resource/resource-schema-v0.json
	@echo "        COPY" $@
	@mkdir -p $(dir $@)
	@cp $< $@

# package: json-schema-for-humans
%.gen.md: %.json json-schema-for-humans-config.yaml
	@echo " JSON SCHEMA" $@
	@generate-schema-doc --config-file json-schema-for-humans-config.yaml $< $@.tmp
	@(head -n 1 $@.tmp; echo "[Raw JSON schema]($(shell basename $<))"; tail -n +2 $@.tmp) \
	  | sed -e 's!a name=!a id=!g' \
		    -e 's!\[\](#[^)]*)!!g' \
				-e 's!^#!##!g' \
				-e 's!\[\[Any type: allowed\]\](# "Additional Properties of any type are allowed\.")!Any type allowed!g' \
				-e 's!\[\[Not allowed\]\](# "Additional Properties not allowed.")!Not allowed!g' \
				> $@
	@rm $@.tmp

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

$(eval $(call cargo_or_provided_exe,NIXOPS4_RESOURCE_RUNNER,nixops4-resource-runner))

src/cli/nixops4-resource-runner.md: $(NIXOPS4_RESOURCE_RUNNER_EXE)
	@echo "  GENERATING" $@
	@mkdir -p $(dir $@)
	@$(NIXOPS4_RESOURCE_RUNNER_CMD) generate-markdown > $@

src/schema/resource-v0/examples/%.json: ../../rust/nixops4-resource/examples/v0/%.json
	@echo "        COPY" $@
	@mkdir -p $(dir $@)
	@cp $< $@
