
default: book

.PHONY: clean default open

include ./make/cargo_or_provided_exe.mk

open: book
	xdg-open book/index.html || open book/index.html

GENERATED = src/schema/resource-schema-v0.gen.md \
	src/cli/nixops4-resource-runner.md \
	src/schema/resource-v0/examples/CreateResourceRequest.json \
	src/schema/resource-v0/examples/CreateResourceResponse.json

clean:
	rm -rf book $(GENERATED)

book: $(shell find src -type f) $(GENERATED) book.toml custom.css mermaid-init.js mermaid.min.js
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

# sets
# - NIXOPS4_RESOURCE_RUNNER_CMD
# - NIXOPS4_RESOURCE_RUNNER_EXE, possibly empty
$(eval $(call cargo_or_provided_exe,NIXOPS4_RESOURCE_RUNNER,nixops4-resource-runner))

src/cli/nixops4-resource-runner.md: $(NIXOPS4_RESOURCE_RUNNER_EXE)
	@echo "  GENERATING" $@
	@mkdir -p $(dir $@)
	@$(NIXOPS4_RESOURCE_RUNNER_CMD) generate-markdown > $@

src/schema/resource-v0/examples/%.json: ../../rust/nixops4-resource/examples/v0/%.json
	@echo "        COPY" $@
	@mkdir -p $(dir $@)
	@cp $< $@
