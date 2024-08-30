
default: book

.PHONY: clean default open

open: book
	xdg-open book/index.html || open book/index.html

GENERATED = src/schema/resource-schema-v0.md

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
%.md: %.json json-schema-for-humans-config.yaml
	@echo " JSON SCHEMA" $@
	@generate-schema-doc --config-file json-schema-for-humans-config.yaml $< $@.tmp
	@(head -n 1 $@.tmp; echo "[Raw JSON schema]($(shell basename $<))"; tail -n +2 $@.tmp) > $@
	@rm $@.tmp
