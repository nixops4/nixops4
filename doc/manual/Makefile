
default: book

.PHONY: clean default open

include ./make/cargo_or_provided_exe.mk

open: book
	xdg-open book/index.html || open book/index.html

GENERATED = src/schema/resource-schema-v0.gen.md \
	src/cli/nixops4-resource-runner.md \
	src/cli/nixops4.md \
	src/modules/option-docs.gen.md \
	src/schema/resource-v0/examples/CreateResourceRequest.json \
	src/schema/resource-v0/examples/CreateResourceResponse.json \
	src/architecture/cargo-deps.gen.md \
	src/lib/lib.gen.md

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

src/architecture/cargo-deps.gen.md: cargo-deps.sh $(shell find ../../ -name Cargo.toml)
	@echo "  CARGO-DEPS" $@
	@mkdir -p $(dir $@)
	@./cargo-deps.sh >$@

$(eval $(call cargo_or_provided_exe,NIXOPS4,nixops4))

src/cli/nixops4.md: $(NIXOPS4_EXE)
	@echo "  GENERATING" $@
	@mkdir -p $(dir $@)
	@$(NIXOPS4_CMD) generate-markdown > $@

src/lib/lib.gen.md: ../../nix/lib/lib.nix Makefile
	@echo "      NIXDOC" $@
	@nixdoc --file $< --category "" --description "" --prefix "" --anchor-prefix "" >src/lib/lib.gen.md

src/modules/option-docs.gen.md: $(shell if [[ -z "$$NIXOPS_DEPLOYMENT_OPTION_DOCS_MD" ]]; then \
			echo deployment-option-docs-md.nix; \
			echo ../../flake.lock; \
			find ../../nix -name '*.nix'; \
		fi)
	@if [[ -z "$$NIXOPS_DEPLOYMENT_OPTION_DOCS_MD" ]]; then \
	    if type -p nix >/dev/null; then \
				rm -f $@; \
				echo "         NIX" $@; \
				nix build --extra-experimental-features 'nix-command flakes' \
				  .#manual-deployment-option-docs-md --out-link $@.tmp; \
				cat $@.tmp > $@; \
				rm $@.tmp; \
			else \
				echo "Nix not found and NIXOPS_DEPLOYMENT_OPTION_DOCS_MD not set. Can't build option docs."; \
				exit 1; \
			fi; \
		else \
			echo "        COPY" $@; \
			cp $$NIXOPS_DEPLOYMENT_OPTION_DOCS_MD $@; \
		fi
