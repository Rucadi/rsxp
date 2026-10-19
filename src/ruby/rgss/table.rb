class Table
  # Portable storage used when the host does not install native Table methods.
  unless method_defined?(:__initialize)
    def __initialize(x, y, z)
      __check_table_size(x, y, z)
      @x, @y, @z = x, y, z
      @data = Array.new(x * y * z, 0)
    end
    def __get(x, y, z)
      return nil unless @data
      return nil if x < 0 || y < 0 || z < 0 || x >= @x || y >= @y || z >= @z
      @data[(z * @y + y) * @x + x]
    end
    def __set(x, y, z, value)
      return unless @data
      return if x < 0 || y < 0 || z < 0 || x >= @x || y >= @y || z >= @z
      @data[(z * @y + y) * @x + x] = ((value + 32768) & 65535) - 32768
    end
    def initialize_copy(other)
      super
      @data = other.instance_variable_get(:@data)&.dup
    end
    def __check_table_size(x, y, z)
      raise ArgumentError, 'Table exceeds the 16 million element limit' if x * y * z > 16_777_216
      raise ArgumentError, 'invalid Table dimensions' if [x, y, z].any? { |n| n > 2_147_483_647 }
    end
    def __resize(x, y, z)
      __check_table_size(x, y, z)
      return unless @data
      old, ox, oy, oz = @data, @x, @y, @z
      @x, @y, @z = x, y, z
      @data = Array.new(x * y * z, 0)
      if old
        [z, oz].min.times do |k|
          [y, oy].min.times do |j|
            [x, ox].min.times do |i|
              @data[(k * y + j) * x + i] = old[(k * oy + j) * ox + i]
            end
          end
        end
      end
    end
    def __dimensions
      @data ? [@x, @y, @z] : [0, 0, 0]
    end
    def _dump(_depth = 0)
      return [1, 0, 1, 1, 0].pack('l<5') unless @data
      dim = @z > 1 ? 3 : (@y > 1 ? 2 : 1)
      [dim, @x, @y, @z, @data.size].pack('l<5') + @data.pack('s<*')
    end
    def self._load(bytes)
      raise ArgumentError, 'bad Table payload' if bytes.bytesize < 20
      dim, x, y, z, count = bytes.unpack('l<5')
      unless (1..3).cover?(dim) && [x, y, z, count].all? { |n| n >= 0 } && x * y * z == count && bytes.bytesize == 20 + count * 2
        raise ArgumentError, 'bad Table payload'
      end
      table = new(x, y, z)
      data = table.instance_variable_get(:@data)
      bytes[20..].unpack('s<*').each_with_index { |value, i| data[i] = value }
      table
    end
  end
  unless instance_methods(false).include?(:initialize) || private_instance_methods(false).include?(:initialize)
    def initialize(x,y=1,z=1) = __initialize(*[x,y,z].map { |v| [Integer(v),0].max })
  end
  def [](x,y=0,z=0) = __get(Integer(x),Integer(y),Integer(z)) unless instance_methods(false).include?(:[])
  unless instance_methods(false).include?(:[]=)
    def []=(*args)
      raise ArgumentError, 'expected one to three coordinates and a value' unless (2..4).cover?(args.size)
      value = Integer(args.pop)
      coords = args.map { |v| Integer(v) }
      coords << 0 while coords.size < 3
      __set(*coords,value)
    end
  end
  def resize(x,y=1,z=1) = __resize(*[x,y,z].map { |v| [Integer(v),0].max }) unless instance_methods(false).include?(:resize)
  def xsize = __dimensions[0] unless instance_methods(false).include?(:xsize)
  def ysize = __dimensions[1] unless instance_methods(false).include?(:ysize)
  def zsize = __dimensions[2] unless instance_methods(false).include?(:zsize)
end

