# frozen_string_literal: true

require 'spec_helper'

# Test the autorouter walk phase directly without loading auto_router.rb
# (which contains Opal-specific JS backtick blocks)
describe 'AutoRouter walk phase' do
  def load_game(fixture_name)
    file = File.join(File.dirname(__FILE__), '..', '..', 'data', 'autorouter', "#{fixture_name}.json")
    data = JSON.parse(File.read(file))
    Engine::Game.load(data)
  end

  # Port of AutoRouter#path — the walk phase only, no JS combo
  def walk_routes(game, path_timeout: 30, route_limit: 10_000)
    corporation = game.current_entity
    trains = game.route_trains(corporation).sort_by(&:price)
    graph = game.graph_for_entity(corporation)

    nodes = graph.connected_nodes(corporation).keys.sort_by do |node|
      revenue = trains.map { |train| node.route_revenue(game.phase, train) }.max
      [
        node.tokened_by?(corporation) ? 0 : 1,
        node.offboard? ? 0 : 1,
        -revenue,
      ]
    end

    connections = {}
    train_routes = Hash.new { |h, k| h[k] = [] }
    next_hexside_bit = 0
    hexside_bits = Hash.new { |h, k| h[k] = 0 }
    timed_out = false
    now = Time.now

    walk_corporation = graph.no_blocking? ? nil : corporation

    total_walk_calls = Hash.new(0)
    total_callbacks = 0

    nodes.each do |node|
      if Time.now - now > path_timeout
        timed_out = true
        break
      end

      walk_calls = Hash.new(0)
      node_callbacks = 0
      node.walk(corporation: walk_corporation, skip_paths: {}, walk_calls: walk_calls) do |_, vp|
        node_callbacks += 1
        total_callbacks += 1
        paths = vp.keys
        chains = []
        chain = []
        left = nil
        right = nil
        last_left = nil
        last_right = nil

        complete = lambda do
          chains << { nodes: [left, right], paths: chain, hexes: chain.map(&:hex) }
          last_left = left
          last_right = right
          left, right = nil
          chain = []
        end

        assign = lambda do |a, b|
          if a && b
            if a == last_left || b == last_right
              left = b
              right = a
            else
              left = a
              right = b
            end
            complete.call
          elsif !left
            left = a || b
          elsif !right
            right = a || b
            complete.call
          end
        end

        paths.each do |path|
          chain << path
          a, b = path.nodes
          assign.call(a, b) if a || b
        end

        if chains.empty?
          next unless left

          chains << { nodes: [left, nil], paths: [] }
          id = [left]
        else
          id = chains.flat_map { |c| c[:paths] }.sort!
        end

        next if connections[id]

        connections[id] = chains.map do |c|
          { left: c[:nodes][0], right: c[:nodes][1], chain: c }
        end

        connection = connections[id]
        path_abort = trains.to_h { |train| [train, true] }

        trains.each do |train|
          route = Engine::Route.new(
            game,
            game.phase,
            train,
            connection_data: connection.clone,
            bitfield: bitfield_from_connection(connection, hexside_bits, next_hexside_bit),
          )
          next_hexside_bit = [@next_hexside_bit_out, next_hexside_bit].max
          route.routes = [route]
          route.revenue(suppress_check_route_combination: true)
          train_routes[train] << route
        rescue Engine::RouteTooLong
          path_abort.delete(train)
        rescue Engine::ReusesCity
          path_abort.clear
        rescue Engine::NoToken, Engine::RouteTooShort, Engine::GameError # rubocop:disable Lint/SuppressedException
        end

        next :abort if path_abort.empty?
      end
      total_walk_calls[:all] += walk_calls[:all]
      total_walk_calls[:not_skipped] += walk_calls[:not_skipped]
      $stderr.puts "    node #{node.hex.id}-#{node.index}: #{node_callbacks} callbacks, walk_calls=#{walk_calls[:all]}/#{walk_calls[:not_skipped]}, paths=#{node.paths.size}"
    end
    $stderr.puts "  Total: #{total_callbacks} callbacks, walk_calls=#{total_walk_calls[:all]}/#{total_walk_calls[:not_skipped]}"

    train_routes.each do |train, routes|
      train_routes[train] = routes.sort_by(&:revenue).reverse.take(route_limit)
    end

    {
      corporation: corporation,
      trains: trains,
      train_routes: train_routes,
      timed_out: timed_out,
      connections_found: connections.size,
    }
  end

  def bitfield_from_connection(connection, hexside_bits, next_hexside_bit)
    @next_hexside_bit_out = next_hexside_bit
    bitfield = [0]
    connection.each do |conn|
      paths = conn[:chain][:paths]
      if paths.size == 1
        if paths[0].nodes[0]
          check_edge_and_set(bitfield, paths[0].nodes[0].id, hexside_bits)
        end
        if paths[0].nodes.size > 1 && paths[0].nodes[1]
          check_edge_and_set(bitfield, paths[0].nodes[1].id, hexside_bits)
        end
      else
        (paths.size - 1).times do |index|
          node1 = paths[index]
          node2 = paths[index + 1]
          case node1.edges.size
          when 1
            check_edge_and_set(bitfield, node1.edges[0].id, hexside_bits)
            check_edge_and_set(bitfield, node2.edges[0].id, hexside_bits)
          when 2
            check_edge_and_set(bitfield, node1.edges[0].id, hexside_bits)
            check_edge_and_set(bitfield, node1.edges[1].id, hexside_bits)
            check_edge_and_set(bitfield, node1.edges[1].id, hexside_bits)
            check_edge_and_set(bitfield, node2.edges[0].id, hexside_bits)
          end
        end
      end
    end
    bitfield
  end

  def check_edge_and_set(bitfield, hexside_edge, hexside_bits)
    @next_hexside_bit_out ||= 0
    if hexside_bits.include?(hexside_edge)
      set_bit(bitfield, hexside_bits[hexside_edge])
    else
      hexside_bits[hexside_edge] = @next_hexside_bit_out
      set_bit(bitfield, @next_hexside_bit_out)
      @next_hexside_bit_out += 1
    end
  end

  def set_bit(bitfield, bit)
    entry = (bit / 32).to_i
    mask = 1 << (bit & 31)
    add_count = entry + 1 - bitfield.size
    while add_count.positive?
      bitfield << 0
      add_count -= 1
    end
    bitfield[entry] |= mask
  end

  def conflicts?(a, b)
    [a.size, b.size].min.times do |i|
      return true if ((a[i] || 0) & (b[i] || 0)) != 0
    end
    false
  end

  def greedy_combo(train_routes)
    best_routes = []
    used_bitfield = []

    train_routes.each do |_train, routes|
      best = routes.find do |route|
        !conflicts?(route.bitfield || [], used_bitfield)
      end
      next unless best

      best_routes << best
      used_bitfield = [used_bitfield.size, (best.bitfield || []).size].max.times.map do |i|
        ((used_bitfield[i] || 0) | ((best.bitfield || [])[i] || 0))
      end
    end

    total = begin
      best_routes.each { |r| r.routes = best_routes }
      best_routes.sum { |r| r.revenue rescue 0 }
    rescue StandardError
      0
    end

    { routes: best_routes, total_revenue: total }
  end

  shared_examples 'walks game' do |fixture_name, expected_corp, timeout: 30|
    it "walks and finds routes for all trains in game #{fixture_name}" do
      game = load_game(fixture_name)
      start = Process.clock_gettime(Process::CLOCK_MONOTONIC)
      result = walk_routes(game, path_timeout: timeout)
      elapsed_ms = ((Process.clock_gettime(Process::CLOCK_MONOTONIC) - start) * 1000).round(1)

      expect(result[:corporation].id).to eq(expected_corp)

      # Every train should have routes (even if walk timed out, partial results are fine)
      result[:trains].each do |train|
        routes = result[:train_routes][train]
        expect(routes).not_to be_empty, "Train #{train.name} should have routes"
      end

      # Greedy combo — may not find routes for all trains on constrained boards
      combo = greedy_combo(result[:train_routes])
      expect(combo[:total_revenue]).to be > 0

      $stderr.puts "\n  Game #{fixture_name} (#{expected_corp}):"
      $stderr.puts "    Ruby walk: #{elapsed_ms}ms, timed_out=#{result[:timed_out]}, connections=#{result[:connections_found]}"
      result[:train_routes].each do |train, routes|
        $stderr.puts "    #{train.name}: #{routes.size} routes, best_rev=#{routes.first&.revenue rescue 'err'}"
      end
      $stderr.puts "    Greedy combo: revenue=#{combo[:total_revenue]}, trains=#{combo[:routes].size}/#{result[:trains].size}"
      combo[:routes].each do |route|
        $stderr.puts "      #{route.train.name}: rev=#{route.revenue rescue 'err'}"
      end
    end
  end

  context 'with game 34 (1870, MP, 2x12T)' do
    include_examples 'walks game', '34', 'MP'
  end

  context 'with game 35 (1870, MKT, 2x12T)' do
    include_examples 'walks game', '35', 'MKT', timeout: 120
  end

  context 'with game 36 (1870, SP, 10T+12T)' do
    include_examples 'walks game', '36', 'SP', timeout: 120
  end

  context 'with game 37 (1870, TP, 1x12T)' do
    include_examples 'walks game', '37', 'TP'
  end

  context 'with game 38 (1870, MKT, 2x12T)' do
    include_examples 'walks game', '38', 'MKT', timeout: 120
  end

  context 'with game 39 (1846, IC, 4/6+7/8)' do
    include_examples 'walks game', '39', 'IC'
  end

  context 'with game 40 (1846, B&O, 4/6+7/8)' do
    include_examples 'walks game', '40', 'B&O'
  end
end
