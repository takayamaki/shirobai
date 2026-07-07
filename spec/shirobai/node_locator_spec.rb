# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::NodeLocator do
  def parse(src)
    RuboCop::ProcessedSource.new(src, RUBY_VERSION.to_f)
  end

  def find_send(ast, name)
    found = nil
    walk = lambda do |node|
      return unless node.is_a?(Parser::AST::Node)

      if node.type == :send && node.children[1] == name
        found = node
      else
        node.children.each { |c| walk.call(c) }
      end
    end
    walk.call(ast)
    found
  end

  def target_of(node)
    expr = node.loc.expression
    [expr.begin_pos, expr.end_pos]
  end

  describe ".locate" do
    context "when the target is a send node inside heredoc interpolation" do
      let(:source) do
        <<~RUBY
          msg = <<~TEXT
            hello \#{user.name}
          TEXT
        RUBY
      end

      it "finds the send node (phase 2 fallback)" do
        processed = parse(source)
        send_node = find_send(processed.ast, :name)
        expect(send_node).not_to be_nil, "expected to find :name send node in AST"

        target = target_of(send_node)
        result = described_class.locate(processed, [target])

        expect(result).to have_key(target)
        expect(result[target].type).to eq(:send)
        expect(result[target].children[1]).to eq(:name)
      end
    end

    context "when the target is a normal (non-heredoc) node" do
      let(:source) { "x = foo.bar(1)" }

      it "finds the send node (phase 1)" do
        processed = parse(source)
        send_node = find_send(processed.ast, :bar)
        expect(send_node).not_to be_nil

        target = target_of(send_node)
        result = described_class.locate(processed, [target])

        expect(result).to have_key(target)
        expect(result[target].children[1]).to eq(:bar)
      end
    end

    context "when locating multiple targets including one in a heredoc" do
      let(:source) do
        <<~RUBY
          x = foo.bar
          msg = <<~TEXT
            value: \#{obj.baz}
          TEXT
        RUBY
      end

      it "finds both nodes (phase 1 for one, phase 2 for the other)" do
        processed = parse(source)
        bar_node = find_send(processed.ast, :bar)
        baz_node = find_send(processed.ast, :baz)
        expect(bar_node).not_to be_nil
        expect(baz_node).not_to be_nil

        bar_target = target_of(bar_node)
        baz_target = target_of(baz_node)
        result = described_class.locate(processed, [bar_target, baz_target])

        expect(result[bar_target].children[1]).to eq(:bar)
        expect(result[baz_target].children[1]).to eq(:baz)
      end
    end

    context "when a wrapper node shares its child's exact range" do
      let(:source) { "foo(a: 1)" }

      def find_node(ast, type)
        found = nil
        walk = lambda do |node|
          return unless node.is_a?(Parser::AST::Node)

          found = node if node.type == type
          node.children.each { |c| walk.call(c) }
        end
        walk.call(ast)
        found
      end

      it "resolves the shared range to the shallowest (wrapper) node" do
        processed = parse(source)

        # A braceless single-pair keyword hash produces a `hash` wrapper whose
        # expression range equals its single `pair` child's (`a: 1`); the
        # shallowest node (the wrapper hash) must win the tie. Its range also
        # contains the target range, so phase 1's prune never skips it.
        hash_node = find_node(processed.ast, :hash)
        expect(hash_node).not_to be_nil

        target = target_of(hash_node)
        result = described_class.locate(processed, [target])

        expect(result).to have_key(target)
        expect(result[target]).to equal(hash_node)
        expect(result[target].type).to eq(:hash)
      end
    end

    context "when a target matches no node in the AST" do
      let(:source) { "x = foo.bar" }

      it "leaves that range absent from the result" do
        processed = parse(source)

        bogus = [1000, 1005]
        result = described_class.locate(processed, [bogus])

        expect(result).not_to have_key(bogus)
        expect(result).to be_empty
      end
    end
  end
end
