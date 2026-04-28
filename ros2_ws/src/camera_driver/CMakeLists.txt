cmake_minimum_required(VERSION 3.8)
project(camera_driver)

if(CMAKE_COMPILER_IS_GNUCXX OR CMAKE_CXX_COMPILER_ID MATCHES "Clang")
	add_compile_options(-Wall -Wextra -Wpedantic)
endif()

find_package(ament_cmake REQUIRED)
find_package(cv_bridge REQUIRED)
find_package(OpenCV REQUIRED)
find_package(rclcpp REQUIRED)
find_package(sensor_msgs REQUIRED)

add_executable(camera_driver_node src/camera_driver_node.cpp)
target_include_directories(camera_driver_node PUBLIC
	$<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
	$<INSTALL_INTERFACE:include>)
ament_target_dependencies(camera_driver_node
	cv_bridge
	rclcpp
	sensor_msgs)
target_link_libraries(camera_driver_node ${OpenCV_LIBS})
target_compile_features(camera_driver_node PUBLIC cxx_std_17)

install(TARGETS camera_driver_node
	DESTINATION lib/${PROJECT_NAME})

install(DIRECTORY config launch
	DESTINATION share/${PROJECT_NAME})

ament_package()
