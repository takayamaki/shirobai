# frozen_string_literal: true

require_relative "../../spec_helper"

RSpec.describe Shirobai::RSpec::NodeLocator do
  def parse(src)
    RuboCop::ProcessedSource.new(src, RUBY_VERSION.to_f)
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

      it "finds the send node" do
        processed = parse(source)
        ast = processed.ast

        # Walk the AST to find the :name send node inside the heredoc
        # interpolation so we can use its exact range as the locate target.
        send_node = nil
        find_send = lambda do |node|
          if node.is_a?(Parser::AST::Node)
            if node.type == :send && node.children[1] == :name
              send_node = node
            else
              node.children.each { |c| find_send.call(c) }
            end
          end
        end
        find_send.call(ast)

        expect(send_node).not_to be_nil, "expected to find :name send node in AST"

        expr = send_node.loc.expression
        target = [expr.begin_pos, expr.end_pos]
        result = described_class.locate(processed, [target])

        expect(result).to have_key(target)
        expect(result[target].type).to eq(:send)
        expect(result[target].children[1]).to eq(:name)
      end
    end

    context "when the target is a normal (non-heredoc) node" do
      let(:source) { "x = foo.bar(1)" }

      it "finds the send node" do
        processed = parse(source)
        ast = processed.ast

        # Find the :bar send node
        send_node = nil
        find_send = lambda do |node|
          if node.is_a?(Parser::AST::Node)
            if node.type == :send && node.children[1] == :bar
              send_node = node
            else
              node.children.each { |c| find_send.call(c) }
            end
          end
        end
        find_send.call(ast)

        expect(send_node).not_to be_nil

        expr = send_node.loc.expression
        target = [expr.begin_pos, expr.end_pos]
        result = described_class.locate(processed, [target])

        expect(result).to have_key(target)
        expect(result[target].type).to eq(:send)
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

      it "finds both nodes" do
        processed = parse(source)
        ast = processed.ast

        bar_node = nil
        baz_node = nil
        find_nodes = lambda do |node|
          if node.is_a?(Parser::AST::Node)
            if node.type == :send && node.children[1] == :bar
              bar_node = node
            elsif node.type == :send && node.children[1] == :baz
              baz_node = node
            else
              node.children.each { |c| find_nodes.call(c) }
            end
          end
        end
        find_nodes.call(ast)

        expect(bar_node).not_to be_nil
        expect(baz_node).not_to be_nil

        bar_expr = bar_node.loc.expression
        baz_expr = baz_node.loc.expression
        bar_target = [bar_expr.begin_pos, bar_expr.end_pos]
        baz_target = [baz_expr.begin_pos, baz_expr.end_pos]

        result = described_class.locate(processed, [bar_target, baz_target])

        expect(result).to have_key(bar_target)
        expect(result).to have_key(baz_target)
        expect(result[bar_target].children[1]).to eq(:bar)
        expect(result[baz_target].children[1]).to eq(:baz)
      end
    end
  end
end
